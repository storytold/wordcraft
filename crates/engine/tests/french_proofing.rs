//! App-level French proofing through the published Pack interface, isolated from unit-test
//! dictionary installs and from the user's Language packs. Every input below is synthetic.
#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};
use wordcraft_doc::{Document, Pos, StoryRef};
use wordcraft_engine::{Session, proof};

const AFF: &str = "SET UTF-8\nFLAG long\nTRY esaitnrulodcpmévqfbghjàxèê\nMAP 2\nMAP eéèê\nMAP aàâ\nSFX S. Y 2\nSFX S. 0 0 .\nSFX S. 0 s .\n";
const DIC: &str = "7\nécole/S.\nmaison/S.\nle/S.\nchat/S.\ndort\net\nla\n";

struct Fixture(PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        proof::packs::set_packs_dir(None);
        proof::set_dictionaries_folder(None);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(s: &mut Session, command: &str, params: Value) -> Value {
    s.run(command, &params).unwrap_or_else(|e| panic!("{command}: {e}"))
}

fn text(s: &Session) -> String {
    s.doc.plain_text(StoryRef::Body)
}

fn fixture() -> (Fixture, PathBuf) {
    let root = std::env::temp_dir().join(format!("wordcraft-french-pack-engine-{}", std::process::id()));
    let pack = root.join("packs/fr");
    let dictionaries = root.join("dictionaries");
    std::fs::create_dir_all(&pack).unwrap();
    std::fs::create_dir_all(&dictionaries).unwrap();
    std::fs::write(pack.join("pack.toml"), "locale = \"fr\"\nname = \"French test\"\nlicence = \"MIT\"\nclass = \"Embeddable\"\n").unwrap();
    std::fs::write(pack.join("spelling.aff"), AFF).unwrap();
    std::fs::write(pack.join("spelling.dic"), DIC).unwrap();
    // Another Pack language must keep its own nonempty JSON key next to French.
    std::fs::write(dictionaries.join("lt_LT.aff"), "SET UTF-8\n").unwrap();
    std::fs::write(dictionaries.join("lt_LT.dic"), "2\nlabas\nrytas\n").unwrap();
    proof::set_dictionaries_folder(Some(dictionaries));
    proof::packs::set_packs_dir(Some(root.join("packs")));
    (Fixture(root), pack)
}

fn spelling_in_layout(s: &mut Session) -> usize {
    s.layout()
        .pages
        .iter()
        .flat_map(|p| &p.items)
        .filter_map(|item| if let wordcraft_engine::layout::Placed::Lines { para, .. } = item { Some(para) } else { None })
        .flat_map(|para| &para.issues)
        .filter(|issue| !issue.2)
        .count()
}

fn review_in_region(tag: &str, expected_tag: &str, pack: &Path) {
    let original = "Le chaat dort et la école dort!";
    let mut s = Session::new(Document::from_text(original));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "review.language", json!({"lang": tag, "default": true}));
    let canonical = if expected_tag == "fr-ca" { "fr-CA" } else { "fr-FR" };
    assert_eq!(s.doc.styles.default_chr.lang.as_deref(), Some(canonical));
    assert_eq!(run(&mut s, "format.state", json!({}))["lang"], canonical);
    assert_eq!(proof::pack_dictionary_file(proof::ProofLang::from_tag(Some(tag))), Some(pack.join("spelling.dic")));
    let issues = run(&mut s, "review.issues", json!({}));
    let issues = issues.as_array().unwrap();
    assert!(issues.iter().all(|i| i["lang"] == expected_tag), "{issues:?}");
    assert!(issues.iter().any(|i| i["text"] == "chaat" && i["kind"] == "spelling" && i["suggestions"].as_array().unwrap().contains(&json!("chat"))));
    assert!(issues.iter().any(|i| i["text"] == "la école" && i["suggestions"].as_array().unwrap().contains(&json!("l’école"))));
    assert_eq!(issues.iter().any(|i| i["text"] == "!" && i["kind"] == "grammar"), expected_tag == "fr-fr");
    run(&mut s, "caret.docStart", json!({}));
    let first = run(&mut s, "review.spelling", json!({}));
    assert_eq!((first["text"].as_str(), first["lang"].as_str()), (Some("chaat"), Some(expected_tag)));
    run(&mut s, "review.applySuggestion", json!({"text": "chat"}));
    assert_eq!(text(&s), "Le chat dort et la école dort!");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s), original);
    // The saved default is a document tag, while proofing results use the Pack's lowercase key.
    let saved = serde_json::to_value(&s.prefs).unwrap();
    let mut restarted = Session::new(Document::new());
    restarted.prefs = serde_json::from_value(saved).unwrap();
    run(&mut restarted, "file.new", json!({}));
    assert_eq!(restarted.doc.styles.default_chr.lang.as_deref(), Some(canonical));
}

#[test]
fn french_packs_through_review_regions_undo_and_reload() {
    let (_fixture, pack) = fixture();
    review_in_region("fr_FR", "fr-fr", &pack);
    review_in_region("fr-CA", "fr-ca", &pack);

    let mut mixed = Document::from_text("chaat labaz.");
    mixed.format_range(&Pos::body(0, 0), &Pos::body(0, 6), &|c| c.lang = Some("fr-FR".into())).unwrap();
    mixed.format_range(&Pos::body(0, 6), &Pos::body(0, 12), &|c| c.lang = Some("lt-LT".into())).unwrap();
    let mut s = Session::new(mixed);
    let mixed_issues = run(&mut s, "review.issues", json!({}));
    let spelling: Vec<_> = mixed_issues.as_array().unwrap().iter().filter(|i| i["kind"] == "spelling").collect();
    assert!(spelling.iter().any(|i| i["text"] == "chaat" && i["lang"] == "fr-fr"), "{mixed_issues}");
    assert!(spelling.iter().any(|i| i["text"] == "labaz" && i["lang"] == "lt-lt"), "{mixed_issues}");
    assert!(spelling.iter().all(|i| i["lang"].as_str().is_some_and(|tag| !tag.is_empty())));

    let mut cached = Session::new(Document::from_text("chaat"));
    Arc::make_mut(&mut cached.doc.styles).default_chr.lang = Some("fr-CA".into());
    assert_eq!(spelling_in_layout(&mut cached), 1);
    let before = proof::dictionary_generation();
    std::fs::write(pack.join("spelling.dic"), DIC.replacen("7\n", "8\n", 1) + "chaat\n").unwrap();
    proof::packs::reload();
    assert_ne!(proof::dictionary_generation(), before);
    assert_eq!(spelling_in_layout(&mut cached), 0, "an open document rechecks after a pack reload");
    std::fs::write(pack.join("spelling.dic"), DIC).unwrap();
    proof::packs::reload();
    assert_eq!(spelling_in_layout(&mut cached), 1, "removing the accepted form restores its spelling mark");

    run(&mut cached, "review.addToDictionary", json!({"word": "chaat"}));
    assert_eq!(spelling_in_layout(&mut cached), 0, "Add to Dictionary also applies to French packs");
}
