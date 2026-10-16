//! Citation form check: Missouri and federal examples from the LawCraft spec.

use serde_json::json;
use wordcraft_engine::Session;

use crate::cmd::store;
use crate::specs;

use super::tests::{FEDERAL, MISSOURI, apply_session, err, law, run};

fn check(s: &mut Session, pack: &str, store_findings: bool) -> Vec<serde_json::Value> {
    let r = run(s, "law.citations.check", json!({"pack": pack, "store": store_findings}));
    r["findings"].as_array().cloned().unwrap_or_default()
}

fn has_rule(findings: &[serde_json::Value], rule_id: &str) -> bool {
    findings.iter().any(|f| f["checkId"] == json!(rule_id))
}

fn insert(s: &mut Session, text: &str) {
    run(s, "text.insert", json!({"text": text}));
}

#[test]
fn missouri_good_citations_have_no_findings() {
    let mut s = law();
    for cite in [
        "Thummel v. King, 570 S.W.2d 679 (Mo. banc 1978)",
        "Thummel v. King, 570 S.W.2d 679, 680 (Mo. banc 1978)",
        "Smith v. Jones, 600 S.W.3d 100 (Mo. App. E.D. 2020)",
        "Smith v. Jones, 590 S.W.3d 200 (Mo. App. W.D. 2019)",
        "Smith v. Jones, 580 S.W.3d 300 (Mo. App. S.D. 2021)",
        "Section 537.080, RSMo 2016",
        "§ 537.080 RSMo",
        "Rule 84.04(d)",
        "(R. at 42)",
        "(J.A. 12)",
    ] {
        insert(&mut s, cite);
        insert(&mut s, "\r");
    }
    let f = check(&mut s, "missouri", false);
    assert!(f.is_empty(), "expected no findings: {f:?}");
}

#[test]
fn missouri_section_symbol_form_is_detected() {
    let packs = crate::citation_pack::bundled_map();
    let mo = packs.get("missouri").expect("missouri pack");
    let rule = mo.rules.iter().find(|r| r.id == "moStatuteForm").expect("moStatuteForm rule");
    for cite in ["§ 537.080 RSMo", "Section 537.080, RSMo 2016"] {
        assert!(rule.detect.is_match(cite), "detect must find `{cite}`");
        assert!(rule.valid.as_ref().is_some_and(|v| v.is_match(cite)), "`{cite}` is the listed valid form");
    }
    assert!(rule.detect.is_match("Sec. 537.080 RSMo"), "detect must find the wrong prefix");
    assert!(!rule.valid.as_ref().is_some_and(|v| v.is_match("Sec. 537.080 RSMo")));
}

#[test]
fn missouri_bad_citations_are_reported() {
    let cases = [
        ("Thummel v. King, 570 S.W. 2d 679 (Mo. banc 1978)", "moCaseReporter"),
        ("Thummel v. King, 570 S. W. 2d 679 (Mo. banc 1978)", "moCaseReporter"),
        ("Thummel v. King, 570 SW2d 679 (Mo. banc 1978)", "moCaseReporterWrong"),
        ("Thummel v. King, 570 S.W.2d 679", "moCaseParenthetical"),
        ("Thummel v. King, 570 S.W.2d 679, 680", "moCaseParenthetical"),
        ("Thummel v. King, 570 S.W.2d 679,680 (Mo. banc 1978)", "moPinCite"),
        ("Thummel v. King, 570 S.W.2d 679 (Mo. Banc 1978)", "moBancCaps"),
        ("Thummel v. King, 570 S.W.2d 679 (Mo. banc)", "moBancMissingYear"),
        ("Smith v. Jones, 600 S.W.3d 100 (Mo. App. 2020)", "moAppDistrict"),
        ("Smith v. Jones, 600 S.W.3d 100 (Mo. App. E.D.)", "moAppMissingYear"),
        ("Smith v. Jones, 590 S.W.3d 200 (Mo. App. W.D.)", "moAppMissingYear"),
        ("Smith v. Jones, 580 S.W.3d 300 (Mo. App. S.D.)", "moAppMissingYear"),
        ("In re Smith, 570 S.W.2d 679", "moCaseParenthetical"),
        ("State ex rel. Roe, 590 S.W.3d 200", "moCaseParenthetical"),
        ("Sec. 537.080 RSMo", "moStatuteForm"),
        ("§ 537.080 , RSMo", "moStatuteForm"),
        ("Rule 84.4", "moRuleForm"),
        ("(R. 42)", "moRecordCite"),
    ];
    for (cite, rule) in cases {
        let mut one = law();
        insert(&mut one, cite);
        let f = check(&mut one, "missouri", false);
        assert!(has_rule(&f, rule), "expected {rule} for `{cite}`: {f:?}");
    }
}

#[test]
fn federal_good_citations_have_no_findings() {
    let mut s = law();
    for cite in [
        "Brown v. Board, 347 U.S. 483 (1954)",
        "Miranda v. Arizona, 384 U.S. 436, 444 (1966)",
        "Obergefell v. Hodges, 576 U.S. 644, 675 (2015)",
        "United States v. Nixon, 418 U.S. 683, 713 (1974)",
        "Roe v. Wade, 410 S. Ct. 113 (1973)",
        "Old case, 123 F.2d 456 (8th Cir. 1941)",
        "Smith v. Jones, 567 F.3d 890 (8th Cir. 2009)",
        "Smith v. Jones, 567 F.3d 890, 891 (8th Cir. 2009)",
        "Smith v. Jones, 567 F.3d 890 (2d Cir. 2009)",
        "Smith v. Jones, 567 F.3d 890 (3d Cir. 2009)",
        "Recent case, 99 F.4th 100 (D.C. Cir. 2024)",
        "Doe v. Roe, 123 F. Supp. 2d 456 (E.D. Mo. 2005)",
        "Doe v. Roe, 200 F. Supp. 3d 789 (S.D.N.Y. 2016)",
        "42 U.S.C. § 1983",
        "Fed. R. App. P. 32(a)",
        "Fed. R. Civ. P. 12(b)(6)",
        "(R. at 12)",
        "(Tr. at 3)",
        "(J.A. 12)",
    ] {
        insert(&mut s, cite);
        insert(&mut s, "\r");
    }
    let f = check(&mut s, "federal", false);
    assert!(f.is_empty(), "expected no findings: {f:?}");
}

#[test]
fn federal_bad_citations_are_reported() {
    let cases = [
        ("Brown v. Board, 347 U.S. 483", "usReporter"),
        ("Roe v. Wade, 410 S.Ct. 113 (1973)", "sCtReporterTight"),
        ("Roe v. Wade, 410 S. Ct. 113", "sCtReporter"),
        ("Smith v. Jones, 567 F3d 890 (8th Cir. 2009)", "federalReporterWrong"),
        ("Old case, 123 F.2d 456 (1941)", "federalCourtParen"),
        ("Smith v. Jones, 567 F.3d 890", "federalCourtParen"),
        ("Smith v. Jones, 567 F.3d 890, 891", "federalCourtParen"),
        ("Smith v. Jones, 567 F.3d 890 (2009)", "federalCourtParen"),
        ("Recent case, 99 F.4th 100 (2024)", "federalCourtParen"),
        ("Smith v. Jones, 567 F.3d 890 (8th Cir.)", "federalCirMissingYear"),
        ("Smith v. Jones, 567 F.3d 890 (2d Cir.)", "federalCirMissingYear"),
        ("Smith v. Jones, 567 F.3d 890 (3d Cir.)", "federalCirMissingYear"),
        ("Smith v. Jones, 567 F.3d 890 (2d Cir. 20090)", "federalCourtParen"),
        ("Smith v. Jones, 567 F.3d 890 (3d Cir. 09)", "federalCourtParen"),
        ("Doe v. Roe, 123 F. Supp. 2d 456 (2005)", "federalCourtParen"),
        ("Doe v. Roe, 200 F. Supp. 3d 789 (2016)", "federalCourtParen"),
        ("Smith v. Jones, 567 F.3d 890 (year 2009)", "federalCourtParen"),
        ("Smith v. Jones, 567 F.3d 890 (8th Cir. 20090)", "federalCourtParen"),
        ("Doe v. Roe, 123 F. Supp. 2d 456 (year 2005)", "federalCourtParen"),
        ("Doe v. Roe, 123 F. Supp. 2d 456 (E.D. Mo. 20050)", "federalCourtParen"),
        ("42 U.S.C. 1983", "uscSection"),
        ("Fed R App P 32(a)", "fedRuleApp"),
        ("Fed. R. Civ. P 12", "fedRuleCiv"),
        ("(Rec. 12)", "federalRecordCite"),
    ];
    for (cite, rule) in cases {
        let mut s = law();
        insert(&mut s, cite);
        let f = check(&mut s, "federal", false);
        assert!(has_rule(&f, rule), "expected {rule} for `{cite}`: {f:?}");
    }
}

#[test]
fn pin_page_range_separators_are_accepted() {
    for dash in ['-', '–', '—'] {
        let mo = format!("Thummel v. King, 570 S.W.2d 679, 680{dash}681 (Mo. banc 1978)");
        let mut s = law();
        insert(&mut s, &mo);
        let f = check(&mut s, "missouri", false);
        assert!(!has_rule(&f, "moCaseParenthetical"), "`{mo}` must be accepted: {f:?}");

        let fed = format!("Smith v. Jones, 567 F.3d 890, 891{dash}892 (8th Cir. 2009)");
        let mut s = law();
        insert(&mut s, &fed);
        let f = check(&mut s, "federal", false);
        assert!(!has_rule(&f, "federalCourtParen"), "`{fed}` must be accepted: {f:?}");

        let us = format!("Brown v. Board, 347 U.S. 483, 484{dash}485 (1954)");
        let mut s = law();
        insert(&mut s, &us);
        let f = check(&mut s, "federal", false);
        assert!(!has_rule(&f, "usReporter"), "`{us}` must be accepted: {f:?}");

        let sct = format!("Roe v. Wade, 410 S. Ct. 113, 114{dash}115 (1973)");
        let mut s = law();
        insert(&mut s, &sct);
        let f = check(&mut s, "federal", false);
        assert!(!has_rule(&f, "sCtReporter"), "`{sct}` must be accepted: {f:?}");
    }
}

#[test]
fn bundled_profiles_reference_citation_packs() {
    let st = store();
    let mo = st.get(MISSOURI).expect("missouri");
    let fed = st.get(FEDERAL).expect("federal");
    assert_eq!(mo.citation_pack.as_deref(), Some("missouri"));
    assert_eq!(fed.citation_pack.as_deref(), Some("federal"));
}

#[test]
fn citation_check_uses_profile_pack_when_applied() {
    let mut s = apply_session(MISSOURI, "brief");
    insert(&mut s, "Thummel v. King, 570 S.W.2d 679");
    let r = run(&mut s, "law.citations.check", json!({}));
    assert_eq!(r["pack"], json!("missouri"));
    assert!(has_rule(r["findings"].as_array().unwrap(), "moCaseParenthetical"));
}

#[test]
fn citation_findings_show_through_proofing() {
    let mut s = apply_session(MISSOURI, "brief");
    insert(&mut s, "Thummel v. King, 570 S.W.2d 679 (Mo. Banc 1978)");
    run(&mut s, "law.citations.check", json!({"pack": "missouri"}));
    let issues = run(&mut s, "review.issues", json!({}));
    assert!(issues.as_array().is_some_and(|a| a.iter().any(|i| i["kind"] == "lint" && i["checkId"] == "moBancCaps")));
    // Stored under the citation check's own source, beside (not over) the court checks'.
    assert!(s.lint_findings_of("law.citations").iter().any(|i| i["checkId"] == "moBancCaps"));
    assert!(run(&mut s, "law.checks.findings", json!({})).as_array().is_some_and(|a| a.iter().all(|i| i["checkId"] != "moBancCaps")));
}

#[test]
fn citation_check_is_pure_and_unknown_pack_fails() {
    let mut s = law();
    insert(&mut s, "text");
    let before = serde_json::to_string(&s.doc).expect("serialize");
    run(&mut s, "law.citations.check", json!({"pack": "missouri", "store": false}));
    let after = serde_json::to_string(&s.doc).expect("serialize");
    assert_eq!(before, after);
    let e = err(&mut s, "law.citations.check", json!({"pack": "no-such-pack"})).to_string();
    assert!(e.contains("no citation pack"), "{e}");
    assert!(specs().iter().any(|c| c.id == "law.citations.check"));
}

#[test]
fn double_space_before_missouri_parenthetical_is_not_a_false_positive() {
    let mut s = law();
    insert(&mut s, "Thummel v. King, 570 S.W.2d 679  (Mo. banc 1978)");
    let f = check(&mut s, "missouri", false);
    assert!(!has_rule(&f, "moCaseParenthetical"), "unexpected findings: {f:?}");
}
