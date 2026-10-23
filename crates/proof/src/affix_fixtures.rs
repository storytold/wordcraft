//! Test fixtures shared by the language suggesters, over the `craft-proof` affix engine.

use crate::affix::AffixDict;

/// Shared hand-written fixture: collectors must reject every path to NOSUGGEST forms.
pub(crate) fn assert_nosuggest_is_honoured(suggest: impl Fn(&AffixDict, &str, usize) -> Vec<String>) {
    let aff = "SET UTF-8\nNOSUGGEST N\nFORBIDDENWORD F\nTRY abeklmnostxy\nMAP 1\nMAP ae\nKEY ak\nREP 2\nREP xy toka\nREP zz normal\nPFX X Y 1\nPFX X 0 pre .\nSFX A Y 1\nSFX A 0 ble/B .\nSFX B Y 1\nSFX B 0 s .\n";
    let d = AffixDict::from_hunspell(aff, "4\ntoka/NXA\nnormal/XA\nTkname/N\nbanned/F\n", &[]).unwrap();
    for word in d.forms("toka").into_iter().chain(["TOKA".into(), "Toka".into(), "Tkname".into(), "TKNAME".into()]) {
        assert!(d.check(&word), "NOSUGGEST is still correct: {word}");
        assert!(!d.can_suggest(&word), "NOSUGGEST form: {word}");
    }
    for input in ["tokax", "tokablesx", "pretokax", "pretokablesx", "toka", "TOKA", "tkname", "teka", "xy", "normaltoka", "toak", "taak"] {
        let results = suggest(&d, input, 32);
        assert!(results.iter().all(|word| word.split(' ').all(|part| d.can_suggest(part))), "{input}: {results:?}");
        assert!(results.iter().all(|word| !word.to_lowercase().contains("toka") && word.to_lowercase() != "tkname"), "{input}: {results:?}");
    }
    assert!(suggest(&d, "norrmal", 8).contains(&"normal".into()));
    assert!(suggest(&d, "zz", 8).contains(&"normal".into()));
    // Filtering happens before display casing: ordinary mixed-case stems retain the
    // existing all-caps suggestion behaviour, with and without a NOSUGGEST declaration.
    for aff in ["", "NOSUGGEST N\n"] {
        let ordinary = AffixDict::from_hunspell(aff, "2\nmixCase\nhidden/N\n", &[]).unwrap();
        assert!(suggest(&ordinary, "MIXCASX", 8).contains(&"MIXCASE".into()));
    }
}
