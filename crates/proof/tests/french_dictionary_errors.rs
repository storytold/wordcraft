//! A separate process keeps the runtime dictionary state out of the synthetic unit tests.

#![cfg(not(target_arch = "wasm32"))]

use wordcraft_proof::{ProofLang, fr};

#[test]
fn unreadable_french_dictionary_reports_and_clears_its_reason() {
    let folder = tempfile::tempdir().unwrap();
    let aff = folder.path().join("fr_FR.aff");
    let dic = folder.path().join("fr_FR.dic");
    std::fs::write(&aff, "SET SHIFT_JIS\n").unwrap();
    std::fs::write(&dic, "1\nmaison\n").unwrap();
    wordcraft_proof::set_dictionaries_folder(Some(folder.path().to_path_buf()));
    assert!(!fr::has_dictionary());
    for tag in ["fr-FR", "fr-CA"] {
        let (name, path, reason) = wordcraft_proof::unreadable_dictionary(Some(tag)).expect("the status bar must explain the error");
        assert_eq!(name, "fr_FR");
        assert_eq!(path, dic);
        assert!(reason.contains("unsupported encoding SHIFT_JIS"), "{reason}");
    }
    for lang in [ProofLang::Fr, ProofLang::FrCa] {
        let reason = wordcraft_proof::dictionary_load_error(lang).expect("explain an unreadable dictionary instead of silently disabling French");
        assert!(reason.contains("unsupported encoding SHIFT_JIS"), "{reason}");
    }
    std::fs::write(&aff, "SET UTF-8\n").unwrap();
    wordcraft_proof::set_dictionaries_folder(Some(folder.path().to_path_buf()));
    assert!(fr::has_dictionary());
    assert!(fr::is_correct("maison"));
    assert!(wordcraft_proof::dictionary_load_error(ProofLang::Fr).is_none());
    assert!(wordcraft_proof::dictionary_load_error(ProofLang::FrCa).is_none());
    assert!(wordcraft_proof::missing_dictionary(Some("fr-FR")).is_none());
    assert!(wordcraft_proof::unreadable_dictionary(Some("fr-CA")).is_none());
    wordcraft_proof::set_dictionaries_folder(None);
    fr::install_dictionary("SET UTF-8\n", "0\n").unwrap();
    for tag in ["fr-FR", "fr-CA"] {
        assert_eq!(wordcraft_proof::missing_dictionary(Some(tag)), Some("fr_FR"));
        assert!(wordcraft_proof::unreadable_dictionary(Some(tag)).is_none());
    }
}
