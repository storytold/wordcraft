//! Opt-in qualification against a real, externally supplied Romanian Hunspell dictionary.
//! No dictionary data is committed or downloaded by this test. Run with
//! `WORDCRAFT_ROMANIAN_TEST_DICT=/path/to/dictionaries cargo test -p wordcraft-proof
//! --test romanian_dictionary -- --ignored --nocapture`.

use unicode_normalization::UnicodeNormalization;
use wordcraft_proof::{ProofLang, check_text, ro, set_dictionaries_folder, suggest_in};

fn issues(text: &str) -> Vec<wordcraft_proof::Issue> {
    check_text(text, &[(0..text.len(), ProofLang::Ro)])
}

#[test]
#[ignore = "requires external ro_RO.aff and ro_RO.dic; see docs/spelling-dictionaries.md"]
fn romanian_external_dictionary_qualifies_text_and_corrections() {
    let folder =
        std::env::var_os("WORDCRAFT_ROMANIAN_TEST_DICT").expect("set WORDCRAFT_ROMANIAN_TEST_DICT to a folder containing ro_RO.aff and ro_RO.dic");
    let folder = std::path::PathBuf::from(folder);
    // Require this exact pair rather than silently finding a different system installation.
    for name in ["ro_RO.aff", "ro_RO.dic"] {
        assert!(folder.join(name).is_file(), "missing {}", folder.join(name).display());
    }
    set_dictionaries_folder(Some(folder.clone()));
    assert!(ro::has_dictionary(), "the supplied Romanian dictionary must load");
    assert!(ro::dictionary().len() > 10_000, "qualification needs a real dictionary, not the unit-test fixture");

    // The shared slot reports files, clears that report for in-memory installs, keeps
    // a working dictionary on parse errors, and reloads the configured folder.
    let path = || wordcraft_proof::installed_dictionaries().into_iter().find(|(l, _)| *l == ProofLang::Ro).unwrap().1;
    assert_eq!(path(), Some(folder.join("ro_RO.dic")));
    assert_eq!(wordcraft_proof::dictionary_load_error(ProofLang::Ro), None);
    ro::install_dictionary("KEEPCASE K\n", "1\nromân/K\n").unwrap();
    assert_eq!(path(), None);
    assert!(ro::install_dictionary("NEEDAFFIX X\n", "1\nromân/X\n").is_err());
    assert!(ro::is_correct("român"), "a refused replacement keeps the in-memory dictionary");
    set_dictionaries_folder(Some(folder.clone()));
    assert!(ro::dictionary().len() > 10_000, "folder reload restores the external dictionary");
    assert_eq!(path(), Some(folder.join("ro_RO.dic")));

    let correct = [
        "Ieri dimineață, fata vecinilor s-a dus într-o librărie din București și și-a cumpărat două cărți despre științele naturii.",
        "Mâine vom merge împreună la munte, dacă nu plouă.",
        "Știința și educația sunt importante pentru copii.",
        "Am ajuns într-o casă din Brașov.",
        "Prețul este de 1.234,56 lei.",
        "Nu este nici un calculator, nici un telefon.",
        "El a parat lovitura.",
        "😀 Știința începe cu întrebări.",
        "Ştiinţa şi educaţia sunt importante pentru copii.",
    ];
    for text in correct {
        assert!(issues(text).is_empty(), "correct text {text:?}: {:?}", issues(text));
        let decomposed: String = text.nfd().collect();
        assert!(issues(&decomposed).is_empty(), "decomposed text {text:?}: {:?}", issues(&decomposed));
    }

    for (wrong, expected) in [("Bucuresti", "București"), ("cîine", "câine"), ("intrun", "într-un"), ("copiiii", "copiii")] {
        let text = format!("😀 Textul de probă: {wrong}.");
        let found = issues(&text);
        let start = text.find(wrong).expect("target in synthetic text");
        let target: Vec<_> = found.iter().filter(|i| i.start == start && i.end == start + wrong.len()).collect();
        assert_eq!(target.len(), 1, "one issue on {wrong}: {found:?}");
        let issue = target[0];
        // Spelling suggestions are deliberately computed on demand, as in the context menu.
        let suggestions = suggest_in(&text[issue.start..issue.end], 5, issue.lang);
        assert!(suggestions.iter().any(|s| s == expected), "{wrong} → {expected}: {suggestions:?}");
        let mut corrected = text.clone();
        corrected.replace_range(issue.start..issue.end, expected);
        assert!(issues(&corrected).is_empty(), "accepted correction {corrected:?}: {:?}", issues(&corrected));
    }
}
