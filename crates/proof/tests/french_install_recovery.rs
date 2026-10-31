//! A failed disk load must recover through the public in-memory installation API.
#![cfg(not(target_arch = "wasm32"))]

use wordcraft_proof::{ProofLang, dictionary_load_error, fr, set_dictionaries_folder};

#[test]
fn valid_french_install_clears_a_previous_load_error_and_failed_install_preserves_state() {
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(folder.path().join("fr_FR.aff"), "SET SHIFT_JIS\n").unwrap();
    std::fs::write(folder.path().join("fr_FR.dic"), "1\nmaison\n").unwrap();
    set_dictionaries_folder(Some(folder.path().to_owned()));
    assert!(!fr::has_dictionary());
    let original = dictionary_load_error(ProofLang::Fr).expect("the invalid disk load is visible");
    assert!(original.contains("unsupported encoding SHIFT_JIS"), "{original}");
    assert_eq!(dictionary_load_error(ProofLang::FrCa), Some(original.clone()));

    // Parsing failure must retain the old diagnostic and dictionary.
    assert!(fr::install_dictionary("CIRCUMFIX X\n", "1\nmaison/X\n").is_err());
    assert_eq!(dictionary_load_error(ProofLang::Fr), Some(original));
    assert!(!fr::has_dictionary());

    fr::install_dictionary("SET UTF-8\n", "1\nmaison\n").unwrap();
    assert!(fr::has_dictionary());
    assert!(fr::is_correct("maison"));
    assert!(!fr::is_correct("maisoon"));
    for lang in [ProofLang::Fr, ProofLang::FrCa] {
        assert_eq!(dictionary_load_error(lang), None, "a successful replacement is readable");
    }
    assert!(fr::install_dictionary("CIRCUMFIX X\n", "1\nvoiture/X\n").is_err());
    assert!(fr::is_correct("maison"), "failed replacement retains the valid dictionary");
    assert!(!fr::is_correct("voiture"));
    assert_eq!(dictionary_load_error(ProofLang::Fr), None);
    set_dictionaries_folder(None);
}
