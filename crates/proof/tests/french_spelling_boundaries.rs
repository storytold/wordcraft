//! Offline spelling regressions: one process owns its miniature dictionaries.
use std::ops::Range;
use wordcraft_proof::{self as proof, IssueKind, Options, ProofLang};

const AFF: &str = "SET UTF-8\nICONV 5\nICONV e\u{301} é\nICONV E\u{301} É\nICONV a\u{303} ã\nICONV ’ '\nICONV a\u{301} á\n";
const FR: &str = "13\nécole\ncafé\nl'école\nporte\nmonnaie\nporte-monnaie\nva\na\nil\nmaison\net\nnormal\nspécial--autorisé\n";
const PT: &str = "2\nnão\nárea\n";

struct Reset;
impl Drop for Reset {
    fn drop(&mut self) {
        proof::remove_word("zxq--personnel");
        proof::fr::reload_dictionary();
        proof::pt::reload_dictionary();
    }
}

fn spelling(text: &str, spans: &[(Range<usize>, ProofLang)]) -> Vec<(usize, usize, ProofLang)> {
    proof::check_text_with(text, spans, &Options::default(), false, false)
        .into_iter()
        .filter(|i| i.kind == IssueKind::Spelling)
        .map(|i| (i.start, i.end, i.lang))
        .collect()
}

#[test]
fn spelling_keeps_combining_marks_and_declines_empty_compound_parts() {
    let _reset = Reset;
    proof::fr::install_dictionary(AFF, FR).unwrap();
    proof::pt::install_dictionary(AFF, PT).unwrap();
    let mut failures = Vec::new();
    let token_cases: &[(&str, &[(usize, usize)])] = &[
        ("e\u{301}cole", &[(0, 7)]),
        ("cafe\u{301}", &[(0, 6)]),
        ("l’e\u{301}cole", &[(0, 11)]),
        ("'a\u{301}'", &[(1, 4)]),
        ("a\u{301}\u{327}", &[(0, 5)]),
        ("\u{301}", &[]),
        ("\u{301} café", &[(3, 8)]),
        ("1\u{301}abc", &[(0, 1), (3, 6)]),
        ("l’\u{301}école", &[(0, 1), (6, 12)]),
        ("un-\u{301}café", &[(0, 2), (5, 10)]),
        ("\u{5b0}", &[]),
        ("1\u{5b0}abc", &[(0, 1), (3, 6)]),
        ("\u{93f}", &[]),
        ("क\u{93f}", &[(0, 6)]),
        ("\u{20dd}", &[]),
        ("a\u{20dd}", &[(0, 4)]),
        ("a\u{200d}b", &[(0, 1), (4, 5)]),
        ("😀\u{301}", &[]),
    ];
    for (text, expected) in token_cases {
        let actual = proof::words(text);
        if actual != *expected {
            failures.push(format!("token {text:?}: {actual:?}, wanted {expected:?}"));
        }
    }
    for lang in [ProofLang::Fr, ProofLang::FrCa] {
        for text in [
            "école",
            "e\u{301}cole",
            "l’école",
            "l’e\u{301}cole",
            "café",
            "cafe\u{301}",
            "porte-monnaie",
            "va-t-il",
            "a-t-il",
            "maison-école",
            "spécial--autorisé",
        ] {
            let actual = spelling(text, &[(0..text.len(), lang)]);
            if !actual.is_empty() {
                failures.push(format!("correct {lang:?} {text:?}: {actual:?}"));
            }
        }
        for text in ["porte--monnaie", "va--t-il", "maison--école", "e\u{301}colle", "cafffe\u{301}"] {
            let actual = spelling(text, &[(0..text.len(), lang)]);
            let expected = vec![(0, text.len(), lang)];
            if actual != expected {
                failures.push(format!("typo {lang:?} {text:?}: {actual:?}, wanted {expected:?}"));
            }
        }
        let text = "école cafe\u{301} xyzxyz porte--monnaie";
        let foreign = text.find("xyzxyz").unwrap();
        let compound = text.find("porte").unwrap();
        let spans = [(0..foreign, lang), (foreign..compound, ProofLang::Other), (compound..text.len(), lang)];
        let actual = spelling(text, &spans);
        if actual != [(compound, text.len(), lang)] {
            failures.push(format!("mixed {lang:?}: {actual:?}"));
        }
        let no_tools = spelling(text, &[(0..text.len(), ProofLang::Other)]);
        assert!(no_tools.is_empty(), "unsupported language remains unmarked");
        let unchanged = text.to_owned();
        let _ = spelling(text, &spans);
        assert_eq!(text, unchanged, "checking preserves original document bytes");
    }
    // A second language reuses its own declared ICONV, without a new normalizer.
    let pt = "na\u{303}o a\u{301}rea";
    let actual = spelling(pt, &[(0..pt.len(), ProofLang::Pt)]);
    if !actual.is_empty() {
        failures.push(format!("Portuguese ICONV: {actual:?}"));
    }
    proof::add_word("zxq--personnel");
    assert!(proof::fr::is_correct("zxq--personnel"), "an explicit user entry still wins");
    proof::fr::install_dictionary("SET UTF-8\n", "0\n").unwrap();
    let off = "e\u{301}colle porte--monnaie";
    assert!(spelling(off, &[(0..off.len(), ProofLang::Fr)]).is_empty(), "no French dictionary means spelling stays off");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
