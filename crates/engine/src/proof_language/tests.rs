use super::*;
use serde_json::json;
use std::sync::Arc;
use wordcraft_doc::{Block, Part, PartKind, props::CharProps};

/// [`choose`] for Cyrillic `text` with the given existing tag, editing language and dictionaries.
fn choice(text: &str, explicit: Option<&str>, editing: Option<&str>, installed: &[&str]) -> Option<String> {
    choose(&Evidence::of(text), Script::Cyrillic, explicit, editing, &installed.iter().map(|t| t.to_string()).collect::<Vec<_>>())
}

#[test]
fn cyrillic_choice_uses_compatible_tag_editing_language_dictionaries_then_fallback() {
    assert_eq!(choice("дом", Some("bg-BG"), Some("uk-UA"), &["ru-RU"]), Some("bg-BG".into()));
    assert_eq!(choice("дом", Some("en-US"), Some("uk-UA"), &["ru-RU"]), Some("uk-UA".into()));
    assert_eq!(choice("дом", Some("en-US"), Some("en-GB"), &["uk-UA", "ru-RU"]), Some("uk-UA".into()));
    assert_eq!(choice("дом", None, None, &[]), Some("ru-RU".into()));
    assert_eq!(choice("їжа", Some("ru-RU"), Some("ru-RU"), &["ru-RU"]), Some("uk-UA".into()));
    assert_eq!(choice("слоў", Some("ru-RU"), None, &[]), Some("be-BY".into()));
    assert_eq!(choice("і", Some("en-US"), None, &[]), None);
    assert_eq!(choice("і", Some("en-US"), Some("be-BY"), &[]), Some("be-BY".into()));
    assert_eq!(choice("ўї", Some("en-US"), None, &["ru-RU"]), None);
    assert_eq!(choice("қазақ", Some("en-US"), None, &["ru-RU"]), None);
}

#[test]
fn latin_keeps_existing_language_and_region() {
    for tag in ["en-GB", "pl-PL", "es-UY"] {
        assert_eq!(choose(&Evidence::default(), Script::Latin, Some(tag), Some("ru-RU"), &[]).as_deref(), Some(tag));
    }
    assert_eq!(choose(&Evidence::default(), Script::Latin, Some("ru-RU"), Some("pl-PL"), &[]).as_deref(), Some("pl-PL"));
}

#[test]
fn opening_normalizes_nested_and_secondary_stories_and_round_trips() {
    let mut doc = Document::from_text("hello русский hello");
    doc.parts.insert(
        7,
        Part {
            kind: PartKind::Header,
            blocks: vec![Arc::new(Block::Para(Paragraph::with_text("русский", CharProps::default())))],
            ..Default::default()
        },
    );
    let mut table = wordcraft_doc::Table::new(1, 1, 400.0);
    table.rows[0].cells[0].blocks = vec![Arc::new(Block::Para(Paragraph::with_text("русский", CharProps::default())))];
    doc.body.push(Arc::new(Block::Table(table)));
    doc.parts.insert(
        8,
        Part {
            kind: PartKind::Footnote,
            blocks: vec![Arc::new(Block::Para(Paragraph::with_text("русский", CharProps::default())))],
            ..Default::default()
        },
    );
    let session = Session::new(doc);
    let cell = &session.doc.body[1].as_table().unwrap().rows[0].cells[0];
    assert_eq!(cell.blocks[0].as_para().unwrap().props_of_char(0).lang.as_deref(), Some("ru-RU"));
    assert_eq!(session.doc.parts[&8].blocks[0].as_para().unwrap().props_of_char(0).lang.as_deref(), Some("ru-RU"));
    let p = session.doc.body[0].as_para().unwrap();
    assert_eq!(p.props_of_char(6).lang.as_deref(), Some("ru-RU"));
    assert_eq!(session.doc.styles.resolve_char(p.props.style.as_deref(), p.props_of_char("hello русский ".len())).lang.as_deref(), Some("en-US"));
    assert_eq!(session.doc.parts[&7].blocks[0].as_para().unwrap().runs[0].props.lang.as_deref(), Some("ru-RU"));
    assert!(!session.dirty);
    let bytes = wordcraft_docx::write(&session.doc).unwrap();
    let reopened = wordcraft_docx::read(&bytes).unwrap();
    let p = reopened.body[0].as_para().unwrap();
    assert_eq!(p.props_of_char(6).lang.as_deref(), Some("ru-RU"));
}

#[test]
fn opening_and_enabling_respect_preferences_and_protection() {
    let mut s = Session::new(Document::new());
    s.prefs.detect_language = false;
    s.set_document(Document::from_text("русский"));
    assert_ne!(s.doc.body[0].as_para().unwrap().runs[0].props.lang.as_deref(), Some("ru-RU"));
    s.run("review.language", &json!({"detect": true})).unwrap();
    assert_eq!(s.doc.body[0].as_para().unwrap().runs[0].props.lang.as_deref(), Some("ru-RU"));
    s.run("edit.undo", &json!({})).unwrap();
    assert_ne!(s.doc.body[0].as_para().unwrap().runs[0].props.lang.as_deref(), Some("ru-RU"));
    let mut doc = Document::from_text("русский");
    doc.settings.protection = Some("readOnly".into());
    s.set_document(doc.clone());
    assert_eq!(s.doc, doc);
}

#[test]
fn no_proof_deleted_text_and_formatting_survive_detection() {
    let mut doc = Document::from_text("русский русский русский");
    let p = doc.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
    p.format(0, "русский".len(), &|c| c.no_proof = Some(true)).unwrap();
    p.format("русский ".len(), "русский русский".len(), &|c| c.del = Some(0)).unwrap();
    p.format("русский русский ".len(), p.len(), &|c| c.bold = Some(true)).unwrap();
    let s = Session::new(doc);
    let p = s.doc.body[0].as_para().unwrap();
    assert_eq!(p.props_of_char(0).lang, None);
    assert_eq!(p.props_of_char("русский ".len()).lang, None);
    let c = p.props_of_char("русский русский ".len());
    assert_eq!(c.lang.as_deref(), Some("ru-RU"));
    assert_eq!(c.bold, Some(true));
}

#[test]
fn formatted_paste_splits_one_run_without_losing_emphasis() {
    let mut s = Session::new(Document::new());
    s.prefs.default_language = Some("ru-RU".into());
    s.run("edit.paste", &json!({"html": "<p><b>hello русский hello</b></p>"})).unwrap();
    let p = s.doc.body[0].as_para().unwrap();
    assert_eq!(p.props_of_char("hello ".len()).lang.as_deref(), Some("ru-RU"));
    assert_eq!(s.doc.styles.resolve_char(p.props.style.as_deref(), p.props_of_char("hello русский ".len())).lang.as_deref(), Some("en-US"));
    assert!(p.run_ranges().all(|(_, c)| c.bold == Some(true)));
}

#[test]
fn patches_and_run_application_scale_with_script_boundaries() {
    let text = "aд".repeat(10_000);
    let mut doc = Document::from_text(&text);
    let p = doc.body[0].as_para().unwrap();
    let changes = patches(&doc, p, 0..p.len(), None, &[], None);
    assert_eq!(changes.len(), 10_000);
    apply(doc.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap(), &changes);
    let p = doc.body[0].as_para().unwrap();
    assert_eq!(p.runs.len(), 20_000);
    assert_eq!(p.runs.iter().map(|r| r.len).sum::<usize>(), text.len());
    // A second pass makes no changes, including across thousands of formatting runs.
    assert!(patches(&doc, p, 0..p.len(), None, &[], None).is_empty());
}

#[test]
fn detection_from_language_dialog_updates_pending_script_without_losing_font() {
    let mut s = Session::new(Document::new());
    s.prefs.detect_language = false;
    s.run("text.insert", &json!({"text": "дом"})).unwrap();
    s.run("format.bold", &json!({})).unwrap();
    s.run("review.language", &json!({"lang": "en-US", "detect": true})).unwrap();
    assert_eq!(s.caret_props().lang.as_deref(), Some("ru-RU"));
    assert_eq!(s.caret_props().bold, Some(true));
    s.run("text.insert", &json!({"text": " house"})).unwrap();
    assert_eq!(s.caret_props().lang.as_deref(), Some("en-US"));
    assert_eq!(s.caret_props().bold, Some(true));
}

#[test]
fn explicit_script_subtags_are_respected() {
    assert_eq!(scripts::of_language("sr-Latn-RS"), Some(Script::Latin));
    assert_eq!(scripts::of_language("sr-Cyrl-RS"), Some(Script::Cyrillic));
    assert_eq!(choose(&Evidence::default(), Script::Latin, Some("sr-Latn-RS"), None, &[]).as_deref(), Some("sr-Latn-RS"));
}

#[test]
fn skipped_foreign_letters_do_not_supply_evidence_for_live_text() {
    for deleted in [false, true] {
        let mut doc = Document::from_text("їдом");
        let p = doc.para_mut(StoryRef::Body, &wordcraft_doc::Path::top(0)).unwrap();
        p.format(0, "ї".len(), &|c| {
            if deleted {
                c.del = Some(0);
            } else {
                c.no_proof = Some(true);
            }
        })
        .unwrap();
        let s = Session::new(doc);
        assert_eq!(s.doc.body[0].as_para().unwrap().props_of_char("ї".len()).lang.as_deref(), Some("ru-RU"));
    }
}

#[test]
fn aliases_and_unknown_latin_tags_are_preserved() {
    for tag in ["pol", "por", "ita", "hrv", "tur", "ces", "cze", "ind", "spa", "deu", "ger", "ca", "fil", "zz"] {
        assert_eq!(choose(&Evidence::default(), Script::Latin, Some(tag), None, &[]).as_deref(), Some(tag));
    }
    assert_eq!(choice("дом", Some("rus"), Some("uk-UA"), &[]).as_deref(), Some("rus"));
    assert_eq!(choice("саха", Some("sah-RU"), None, &[]).as_deref(), Some("sah-RU"));
    assert_eq!(choice("қазақ", Some("en-US"), Some("kk-KZ"), &[]).as_deref(), Some("kk-KZ"));
    assert_eq!(choice("ђак", Some("en-US"), None, &["sr-RS"]).as_deref(), Some("sr-RS"));
    assert_eq!(crate::speech::speech_language("дом", Some("rus"), None).as_deref(), Some("ru"));
}

#[test]
fn pending_language_follows_same_script_correction_but_keeps_manual_choices() {
    let mut s = Session::new(Document::new());
    s.prefs.default_language = Some("ru-RU".into());
    s.run("review.language", &json!({"lang": "ru-RU"})).unwrap();
    s.run("format.bold", &json!({})).unwrap();
    s.run("text.insert", &json!({"text": "їжа"})).unwrap();
    assert_eq!(s.caret_props().lang.as_deref(), Some("uk-UA"));
    assert_eq!(s.caret_props().bold, Some(true));
    let mut s = Session::new(Document::from_text("дом"));
    s.run("review.language", &json!({"lang": "be-BY", "detect": true})).unwrap();
    assert_eq!(s.caret_props().lang.as_deref(), Some("be-BY"));
}

#[test]
fn table_style_language_and_no_proof_are_resolved_before_detection() {
    for no_proof in [false, true] {
        let mut doc = Document::new();
        let style = doc.styles.get_mut("TableGrid").unwrap();
        style.chr.lang = Some("uk-UA".into());
        style.chr.no_proof = Some(no_proof);
        let mut table = wordcraft_doc::Table::new(1, 1, 400.0);
        table.rows[0].cells[0].blocks = vec![Arc::new(Block::Para(Paragraph::with_text("дом", CharProps::default())))];
        doc.body = vec![Arc::new(Block::Table(table))];
        let s = Session::new(doc);
        let p = s.doc.body[0].as_table().unwrap().rows[0].cells[0].blocks[0].as_para().unwrap();
        assert!(p.props_of_char(0).lang.is_none(), "the inherited Ukrainian tag must not be overwritten");
    }
}

#[test]
fn conditional_table_proofing_uses_one_cached_grid_context() {
    let mut doc = Document::new();
    let style = doc.styles.get_mut("TableGrid").unwrap();
    let mut first = style.table_part(wordcraft_doc::styles::TablePart::FirstRow);
    first.chr.no_proof = Some(true);
    style.set_table_part(wordcraft_doc::styles::TablePart::FirstRow, first);
    let mut table = wordcraft_doc::Table::new(2_000, 2, 400.0);
    table.props.look.header_row = true;
    for row in &mut table.rows {
        for cell in &mut row.cells {
            cell.blocks = vec![Arc::new(Block::Para(Paragraph::with_text("дом", CharProps::default())))];
        }
    }
    doc.body = vec![Arc::new(Block::Table(table))];
    let s = Session::new(doc);
    let table = s.doc.body[0].as_table().unwrap();
    assert!(table.rows[0].cells[0].blocks[0].as_para().unwrap().props_of_char(0).lang.is_none());
    for row in table.rows.iter().skip(1) {
        assert_eq!(row.cells[1].blocks[0].as_para().unwrap().props_of_char(0).lang.as_deref(), Some("ru-RU"));
    }
}
