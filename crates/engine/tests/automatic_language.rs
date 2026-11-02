//! Isolated process: these scenarios replace global dictionary settings and the Russian slot.
use serde_json::json;
use wordcraft_doc::{Document, StoryRef};
use wordcraft_engine::Session;

/// #3353: installed dictionaries choose the Cyrillic tag, and Russian misspellings are reported
/// once the text is tagged Russian.
#[test]
fn dictionaries_choose_tags_and_russian_spelling_uses_the_detected_language() {
    let dir = std::env::temp_dir().join(format!("wordcraft-auto-language-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for name in ["uk_UA", "ru_RU", "be_BY"] {
        std::fs::write(dir.join(format!("{name}.aff")), "SET UTF-8\n").unwrap();
        std::fs::write(dir.join(format!("{name}.dic")), "1\nдом\n").unwrap();
    }
    wordcraft_proof::set_dictionaries_folder(Some(dir.clone()));
    let tags = wordcraft_proof::installed_cyrillic_languages();
    assert_eq!(&tags[..3], ["be-BY", "ru-RU", "uk-UA"]);
    let mut s = Session::new(Document::new());
    s.prefs.default_language = Some("uk-UA".into());
    s.run("text.insert", &json!({"text": "дом"})).unwrap();
    assert_eq!(s.doc.body[0].as_para().unwrap().props_of_char(0).lang.as_deref(), Some("uk-UA"));
    // A folder change invalidates discovery; an unsupported Ukrainian dictionary still sets uk.
    std::fs::remove_file(dir.join("be_BY.aff")).unwrap();
    std::fs::remove_file(dir.join("ru_RU.aff")).unwrap();
    wordcraft_proof::set_dictionaries_folder(Some(dir.clone()));
    let mut s = Session::new(Document::new());
    s.run("text.insert", &json!({"text": "дом"})).unwrap();
    assert_eq!(s.doc.body[0].as_para().unwrap().props_of_char(0).lang.as_deref(), Some("uk-UA"));
    assert!(!wordcraft_proof::has_dictionary(Some("uk-UA")), "discovery must not imply proofing support");
    wordcraft_proof::set_dictionaries_folder(None);
    std::fs::remove_dir_all(&dir).unwrap();

    wordcraft_proof::ru::install_dictionary("SET UTF-8\n", "0\n").unwrap();
    let mut s = Session::new(Document::new());
    s.prefs.detect_language = false;
    s.prefs.default_language = Some("ru-RU".into());
    s.run("text.insert", &json!({"text": "превет"})).unwrap();
    let issues = s.run("review.issues", &json!({})).unwrap();
    assert!(!issues.as_array().unwrap().iter().any(|i| i["kind"] == "spelling"));
    s.run("review.language", &json!({"detect": true})).unwrap();
    let issues = s.run("review.issues", &json!({})).unwrap();
    assert!(!issues.as_array().unwrap().iter().any(|i| i["kind"] == "spelling"));

    wordcraft_proof::ru::install_dictionary("SET UTF-8\n", "1\nпривет\n").unwrap();
    let issues = s.run("review.issues", &json!({})).unwrap();
    assert!(issues.as_array().unwrap().iter().any(|i| i["kind"] == "spelling" && i["text"] == "превет" && i["lang"] == "ru"));
    let p = s.doc.body[0].as_para().unwrap();
    let runs = wordcraft_layout::para::proof_runs(&s.doc, p, None);
    let layout_issues = wordcraft_layout::para::paragraph_issues(p, &runs, &s.prefs.spelling(), true, false);
    assert!(layout_issues.iter().any(|i| i.kind == wordcraft_proof::IssueKind::Spelling));
    assert_eq!(s.doc.plain_text(StoryRef::Body), "превет");
}
