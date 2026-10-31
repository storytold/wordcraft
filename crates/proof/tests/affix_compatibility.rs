//! Frozen public output from the pre-French shared affix engine at upstream
//! e6c5a701ddff5be16e266028168c70658aae39d9, using original synthetic fixtures.
//! Do not regenerate expectations from the new implementation: they establish the baseline.

use std::fmt::Write;
use wordcraft_proof::affix::{AffixDict, capitalize};

#[path = "fixtures/affix_compatibility_cases.rs"]
mod fixtures;

fn snapshot(case: &fixtures::Case, dict: &AffixDict) -> String {
    let mut out = format!("{}\nlen={} rules={}\n", case.name, dict.len(), dict.rule_count());
    writeln!(out, "try={:?} key={:?} map={:?} rep={:?} comments={:?}", dict.try_chars(), dict.key(), dict.map(), dict.rep(), dict.comments())
        .unwrap();
    let mut stems: Vec<_> = dict.stems().collect();
    stems.sort_unstable();
    for stem in &stems {
        let mut forms = dict.forms(stem);
        forms.sort_unstable();
        forms.dedup();
        writeln!(out, "forms {stem:?}: {forms:?}").unwrap();
    }
    let mut probes: Vec<String> = case.probes.iter().map(|word| (*word).to_string()).collect();
    for stem in stems {
        probes.extend(dict.forms(stem));
    }
    let originals = probes.clone();
    for word in originals {
        probes.extend([word.to_uppercase(), word.to_lowercase(), capitalize(&word)]);
    }
    probes.sort_unstable();
    probes.dedup();
    for word in probes {
        writeln!(
            out,
            "{word:?}: contains={} check={} suggest={} raw_suggest={}",
            dict.contains(&word),
            dict.check(&word),
            dict.can_suggest(&word),
            dict.contains_suggestible(&word)
        )
        .unwrap();
    }
    out
}

#[test]
fn non_french_affix_outputs_match_pre_french_baseline() {
    let expected = include_str!("fixtures/affix_compatibility.expected");
    let mut actual = String::new();
    for case in fixtures::cases() {
        let dict = AffixDict::from_hunspell(&case.aff, &case.dic, &["original compatibility fixture"]).unwrap();
        let result = snapshot(&case, &dict);
        let restored = AffixDict::from_bytes(&dict.to_bytes()).unwrap();
        assert_eq!(snapshot(&case, &restored), result, "current compact round trip: {}", case.name);
        actual.push_str(&result);
    }
    assert_eq!(actual, expected, "shared engine output changed from upstream e6c5a70");
}

#[test]
fn pre_french_compact_dictionaries_keep_the_same_outputs() {
    let expected = include_str!("fixtures/affix_compatibility.expected");
    let encoded = include_str!("fixtures/affix_compatibility.compact");
    let cases = fixtures::cases();
    let lines: Vec<_> = encoded.lines().collect();
    assert_eq!(lines.len(), cases.len());
    let mut actual = String::new();
    for (case, hex) in cases.iter().zip(lines) {
        assert_eq!(hex.len() % 2, 0);
        let bytes: Vec<_> =
            hex.as_bytes().as_chunks::<2>().0.iter().map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap()).collect();
        let restored = AffixDict::from_bytes(&bytes).unwrap();
        actual.push_str(&snapshot(case, &restored));
    }
    assert_eq!(actual, expected, "reading old WCAFF1 bytes changed the observable dictionary");
}
