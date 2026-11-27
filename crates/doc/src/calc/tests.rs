use super::*;
use crate::para::InlineObject;

fn calc_equation() -> InlineObject {
    InlineObject::Equation {
        linear: "σ_max=P/A".into(),
        display: true,
        math: Default::default(),
        calc: Some(Box::new(CalcRegion {
            result_format: None,
            worksheet_order: None,
            id: 7,
            source: "σ_max := P / A".into(),
            unit: Some("ksi".into()),
            precision: Some(2),
            numbered: true,
            state: CalcState::Error("undefined: P".into()),
        })),
    }
}

fn spec(name: &str, unit: Option<&str>, precision: Option<u8>, index: Option<(u32, Option<u32>)>) -> QuoteSpec {
    QuoteSpec { name: name.into(), unit: unit.map(str::to_string), precision, index }
}

#[test]
fn instr_round_trip() {
    let instr = calc_field_instr(&spec("σ_max", Some("ksi"), Some(2), None));
    assert_eq!(instr, "WCCALC σ_max \\u ksi \\p 2");
    assert_eq!(parse_calc_field(&instr), Some(spec("σ_max", Some("ksi"), Some(2), None)));
    assert_eq!(parse_calc_field("PAGE"), None);
    assert_eq!(parse_calc_field("WCCALCX a"), None);
    let huge = format!("WCCALC a \\u {}", "m".repeat(10_000));
    assert_eq!(parse_calc_field(&huge), None);
}

#[test]
fn instr_switches_are_optional() {
    assert_eq!(calc_field_instr(&spec("x", None, None, None)), "WCCALC x");
    assert_eq!(parse_calc_field("WCCALC x"), Some(spec("x", None, None, None)));
    assert_eq!(calc_field_instr(&spec("x", None, Some(0), None)), "WCCALC x \\p 0");
    assert_eq!(parse_calc_field("WCCALC x \\p 0"), Some(spec("x", None, Some(0), None)));
    assert_eq!(parse_calc_field("WCCALC x \\p 15"), Some(spec("x", None, Some(15), None)));
    // Switch order doesn't matter.
    assert_eq!(parse_calc_field("WCCALC x \\p 4 \\u m"), Some(spec("x", Some("m"), Some(4), None)));
    assert_eq!(parse_calc_field("WCCALC D \\i 1,2 \\p 4"), Some(spec("D", None, Some(4), Some((1, Some(2))))));
}

#[test]
fn instr_unit_with_spaces_is_quoted() {
    let instr = calc_field_instr(&spec("M", Some("kN m"), None, None));
    assert_eq!(instr, "WCCALC M \\u \"kN m\"");
    assert_eq!(parse_calc_field(&instr), Some(spec("M", Some("kN m"), None, None)));
}

#[test]
fn quote_spec_round_trips() {
    // Every combination of unit, precision and index reads back as written.
    for unit in [None, Some("in"), Some("kN m"), Some("°F")] {
        for precision in [None, Some(0), Some(3), Some(15)] {
            for index in [None, Some((0, None)), Some((2, None)), Some((1, Some(2))), Some((0, Some(0))), Some((u32::MAX, Some(u32::MAX)))] {
                let s = spec("D_o", unit, precision, index);
                let instr = calc_field_instr(&s);
                assert_eq!(parse_calc_field(&instr), Some(s), "{instr}");
            }
        }
    }
    assert_eq!(calc_field_instr(&spec("D", None, None, Some((2, None)))), "WCCALC D \\i 2");
    assert_eq!(calc_field_instr(&spec("M", None, None, Some((1, Some(2))))), "WCCALC M \\i 1,2");
    // An index is one whole number, or a row and a column, given once.
    for bad in [
        "WCCALC D \\i x",
        "WCCALC D \\i 1,2,3",
        "WCCALC D \\i 1 \\i 2",
        "WCCALC D \\i",
        "WCCALC D \\i ,2",
        "WCCALC D \\i 1,",
        "WCCALC D \\i -1",
        "WCCALC D \\i +1",
        "WCCALC D \\i 1.5",
        "WCCALC D \\i 4294967296",
        "WCCALC D \\i 1,4294967296",
    ] {
        assert_eq!(parse_calc_field(bad), None, "{bad:?}");
    }
}

#[test]
fn instr_rejects_malformed() {
    for bad in [
        "WCCALC",
        "WCCALC   ",
        "WCCALC \\u ksi",
        "WCCALC x \\z 1",
        "WCCALC x \\u",
        "WCCALC x \\p",
        "WCCALC x \\p 16",
        "WCCALC x \\p -1",
        "WCCALC x \\p 256",
        "WCCALC x \\p two",
        "WCCALC x \\u a \\u b",
        "WCCALC x \\p 1 \\p 2",
        "WCCALC x extra",
        "WCCALC x \\u \"unterminated",
        "WCCALC x \\u \"\"",
        "WCCALC x \\u \\p",
        "WCCALC x \\u \\p 2",
        "WCCALC \"x y\"",
        "",
    ] {
        assert_eq!(parse_calc_field(bad), None, "{bad:?}");
    }
}

#[test]
fn instr_just_under_limit_parses_and_over_is_rejected() {
    let head = "WCCALC a \\u ";
    let ok = format!("{head}{}", "m".repeat(MAX_INSTR_BYTES - head.len()));
    assert_eq!(ok.len(), MAX_INSTR_BYTES);
    assert!(parse_calc_field(&ok).is_some());
    assert_eq!(parse_calc_field(&format!("{ok}m")), None);
}

#[test]
fn instr_never_panics_on_hostile_input() {
    for s in ["\"", "\\", "WCCALC \\", "WCCALC \"", "WCCALC a \\u \"é", "WCCALC a \\p ٣", "\u{0}\u{0}", "WCCALC a\u{a0}\\p 1"] {
        let _ = parse_calc_field(s);
    }
}

#[test]
fn settings_default() {
    let s = CalcSettings::default();
    assert_eq!((s.system, s.precision, s.manual), (UnitSystem::UsCustomary, 3, false));
    assert_eq!(CalcState::default(), CalcState::Fresh);
}

#[test]
fn plain_equation_serde_unchanged() {
    let plain = InlineObject::Equation { linear: "a=b".into(), display: false, math: Default::default(), calc: None };
    let json = serde_json::to_string(&plain).unwrap();
    assert!(!json.contains("calc"), "{json}");
    // And a file written before the field existed still loads.
    let back: InlineObject = serde_json::from_str(&json).unwrap();
    assert_eq!(back, plain);
    let legacy: InlineObject = serde_json::from_str(r#"{"type":"equation","linear":"a=b"}"#).unwrap();
    assert!(matches!(legacy, InlineObject::Equation { calc: None, display: false, .. }));
}

#[test]
fn calc_equation_serde_round_trip() {
    let eq = calc_equation();
    let json = serde_json::to_string(&eq).unwrap();
    assert!(json.contains("calc"), "{json}");
    let back: InlineObject = serde_json::from_str(&json).unwrap();
    assert_eq!(back, eq);
    for state in [CalcState::Fresh, CalcState::Stale, CalcState::Error("x".into()), CalcState::EditedOutside] {
        let region = CalcRegion {
            result_format: None,
            worksheet_order: None,
            id: 1,
            source: "a:=1".into(),
            unit: None,
            precision: None,
            numbered: false,
            state,
        };
        let back: CalcRegion = serde_json::from_str(&serde_json::to_string(&region).unwrap()).unwrap();
        assert_eq!(back, region);
    }
}

#[test]
fn settings_serde_round_trip_and_partial_load() {
    let s = CalcSettings { result_format: None, system: UnitSystem::Si, precision: 6, manual: true, g: GMeans::Gram, origin: 1 };
    assert_eq!(serde_json::from_str::<CalcSettings>(&serde_json::to_string(&s).unwrap()).unwrap(), s);
    assert_eq!(serde_json::from_str::<CalcSettings>("{}").unwrap(), CalcSettings::default());
}

#[test]
fn document_calc_settings_serde() {
    let mut doc = crate::Document::new();
    let plain = serde_json::to_string(&doc).unwrap();
    assert!(!plain.contains("calc_settings"), "an untouched document stays as it was");
    doc.calc_settings = Some(CalcSettings { result_format: None, system: UnitSystem::Si, precision: 5, manual: true, g: GMeans::Gram, origin: 0 });
    let back: crate::Document = serde_json::from_str(&serde_json::to_string(&doc).unwrap()).unwrap();
    assert_eq!(back.calc_settings, doc.calc_settings);
    assert!(!back.same_content(&crate::Document::new()), "calc settings are part of the document's content");
}

#[test]
fn origin_round_trips_and_defaults() {
    assert_eq!(CalcSettings::default().origin, 0);
    // A Stage 1 settings object has no origin: it numbers from 0.
    let stage1: CalcSettings = serde_json::from_str(r#"{"system":"Si","precision":5,"manual":true,"g":"gram"}"#).unwrap();
    assert_eq!(stage1.origin, 0);
    assert_eq!((stage1.system, stage1.precision, stage1.manual, stage1.g), (UnitSystem::Si, 5, true, GMeans::Gram));
    // The default isn't written, so a Stage 1 document saves as it did.
    assert!(!serde_json::to_string(&CalcSettings::default()).unwrap().contains("origin"));
    let one = CalcSettings { origin: 1, ..CalcSettings::default() };
    let json = serde_json::to_string(&one).unwrap();
    assert!(json.contains("\"origin\":1"), "{json}");
    assert_eq!(serde_json::from_str::<CalcSettings>(&json).unwrap(), one);
}

#[test]
fn store_tables_default_empty_and_are_skipped_when_empty() {
    // A Stage 1 store has no tables: still valid, and saved again as it was.
    let stage1 =
        r#"{"settings":{"system":"UsCustomary","precision":3,"manual":false,"g":"gravity"},"regions":[{"id":1,"source":"a := 1","fingerprint":7}]}"#;
    let store: CalcStore = serde_json::from_str(stage1).unwrap();
    assert!(store.tables.is_empty());
    assert_eq!(serde_json::to_string(&store).unwrap(), stage1);
    // Tables are written when there are any, and count against emptiness.
    let with = CalcStore { tables: vec![StoredTable { id: 2, fingerprint: 9 }], ..CalcStore::default() };
    assert!(!with.is_empty());
    let json = serde_json::to_string(&with).unwrap();
    assert!(json.contains(r#""tables":[{"id":2,"fingerprint":9}]"#), "{json}");
    assert_eq!(serde_json::from_str::<CalcStore>(&json).unwrap(), with);
    assert!(CalcStore::default().is_empty());
    assert!(CALC_TABLE_TAG_PREFIX.strip_prefix(CALC_TAG_PREFIX).is_none() && CALC_TAG_PREFIX.strip_prefix(CALC_TABLE_TAG_PREFIX).is_none());
}
