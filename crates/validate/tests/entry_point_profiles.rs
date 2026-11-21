//! Entry-point profiles require agreement, independently of relationship order or target identity.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[allow(dead_code)]
mod common;

use common::*;
use wordcraft_validate::{Report, Verdict, validate_bytes};

const TRANSITIONAL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const STRICT: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships";
const STRICT_W: &str = "http://purl.oclc.org/ooxml/wordprocessingml/main";
const DANGLING: &str = r#"<w:p><w:r><w:footnoteReference w:id="9"/></w:r></w:p>"#;

fn relationship(id: &str, namespace: &str, kind: &str, target: &str) -> String {
    format!(r#"<Relationship Id="{id}" Type="{namespace}/{kind}" Target="{target}"/>"#)
}

fn fixture(body: &str, profiles: &[&str], distinct: bool) -> Doc {
    let mut d = Doc::minimal();
    d.set_document(body);
    let rels: String = profiles
        .iter()
        .enumerate()
        .map(|(i, profile)| {
            let target = if distinct && i == 1 { "word/other.xml" } else { "word/document.xml" };
            relationship(&format!("r{i}"), profile, "officeDocument", target)
        })
        .collect();
    d.set("_rels/.rels", rels_xml(&rels));
    if distinct {
        // Only one main content type: this exercises root ambiguity without CT-DUP-MAIN.
        d.set("word/other.xml", document_xml(body));
    }
    d
}

fn json(report: &Report) -> Vec<u8> {
    serde_json::to_vec(report).unwrap()
}

fn no_document_verdict(report: &Report) {
    assert_eq!(report.verdict, Verdict::Incomplete, "{report:?}");
    assert!(report.findings.is_empty(), "{report:?}");
    assert!(!report.checks_run.contains(&"REF-NOTE"), "{report:?}");
}

fn mixed_both_orders(body: &str) {
    let mut reports = Vec::new();
    for profiles in [[TRANSITIONAL, STRICT], [STRICT, TRANSITIONAL]] {
        let report = validate_bytes(&fixture(body, &profiles, false).build());
        no_document_verdict(&report);
        assert!(
            report
                .coverage_notes
                .iter()
                .any(|n| n.contains("mixed") && n.contains("Transitional") && n.contains("Strict") && n.contains("entry point")),
            "{report:?}"
        );
        reports.push(json(&report));
    }
    assert_eq!(reports[0], reports[1]);
}

#[test]
fn mixed_profiles_empty_document_is_incomplete_in_both_orders() {
    mixed_both_orders("<w:p/>");
}

#[test]
fn mixed_profiles_dangling_note_is_incomplete_in_both_orders() {
    mixed_both_orders(DANGLING);
}

#[test]
fn mixed_profile_after_two_agreeing_entries_is_still_uncertain() {
    for profiles in [[TRANSITIONAL, TRANSITIONAL, STRICT], [STRICT, STRICT, TRANSITIONAL]] {
        let report = validate_bytes(&fixture(DANGLING, &profiles, false).build());
        no_document_verdict(&report);
        assert!(report.coverage_notes.iter().any(|n| n.contains("mixed")), "{report:?}");
    }
}

#[test]
fn agreeing_transitional_entries_match_single_entry_scans() {
    for body in ["<w:p/>", DANGLING] {
        let single = validate_bytes(&fixture(body, &[TRANSITIONAL], false).build());
        assert_eq!(single.verdict, if body == DANGLING { Verdict::Fail } else { Verdict::Pass });
        assert_eq!(single.findings.iter().any(|f| f.rule == "REF-NOTE"), body == DANGLING);
        assert!(single.coverage_notes.is_empty(), "{single:?}");
        let mut reports = Vec::new();
        for reverse in [false, true] {
            let mut d = fixture(body, &[TRANSITIONAL, TRANSITIONAL], false);
            let a = relationship("a", TRANSITIONAL, "officeDocument", "word/document.xml");
            let b = relationship("b", TRANSITIONAL, "officeDocument", "word/./document.xml");
            d.set("_rels/.rels", rels_xml(&(if reverse { b + &a } else { a + &b })));
            let report = validate_bytes(&d.build());
            assert_eq!(json(&single), json(&report));
            reports.push(json(&report));
        }
        assert_eq!(reports[0], reports[1]);
    }
}

#[test]
fn agreeing_strict_entries_match_single_entry_gate_in_both_orders() {
    for body in ["<w:p/>", DANGLING] {
        for strict_markup in [false, true] {
            let mut single = fixture(body, &[STRICT], false);
            if strict_markup {
                single.set_document_raw(&format!(r#"<w:document xmlns:w="{STRICT_W}"><w:body>{body}</w:body></w:document>"#));
            }
            let expected = validate_bytes(&single.build());
            no_document_verdict(&expected);
            assert!(expected.coverage_notes.iter().any(|n| n.contains("Strict") && n.contains("unsupported")));
            for reverse in [false, true] {
                let mut d = Doc { entries: single.entries.clone() };
                let a = relationship("a", STRICT, "officeDocument", "word/document.xml");
                let b = relationship("b", STRICT, "officeDocument", "word/./document.xml");
                d.set("_rels/.rels", rels_xml(&(if reverse { b + &a } else { a + &b })));
                assert_eq!(json(&expected), json(&validate_bytes(&d.build())));
            }
        }
    }
}

#[test]
fn distinct_mixed_main_targets_keep_competing_root_treatment_in_both_orders() {
    for body in ["<w:p/>", DANGLING] {
        let mut reports = Vec::new();
        for reverse in [false, true] {
            let mut d = fixture(body, &[TRANSITIONAL, STRICT], true);
            if reverse {
                let a = relationship("r0", TRANSITIONAL, "officeDocument", "word/document.xml");
                let b = relationship("r1", STRICT, "officeDocument", "word/other.xml");
                d.set("_rels/.rels", rels_xml(&(b + &a)));
            }
            let report = validate_bytes(&d.build());
            no_document_verdict(&report);
            assert!(
                report.coverage_notes.iter().any(|n| n.contains("ambiguous officeDocument") && n.contains("distinct official targets")),
                "{report:?}"
            );
            assert!(report.checks_run.contains(&"XML-MALFORMED"), "{report:?}");
            reports.push(json(&report));
        }
        assert_eq!(reports[0], reports[1]);
    }
}

#[test]
fn foreign_and_external_relationships_do_not_vote_on_entry_point_profile() {
    for reverse in [false, true] {
        let expected = validate_bytes(&fixture(DANGLING, &[TRANSITIONAL], false).build());
        let mut d = fixture(DANGLING, &[TRANSITIONAL], false);
        let a = relationship("main", TRANSITIONAL, "officeDocument", "word/document.xml");
        let b = relationship("foreign", "https://example.test", "officeDocument", "word/document.xml")
            + &format!(
                r#"<Relationship Id="external" Type="{STRICT}/officeDocument" Target="https://example.test/main.xml" TargetMode="External"/>"#
            );
        d.set("_rels/.rels", rels_xml(&(if reverse { b + &a } else { a + &b })));
        assert_eq!(json(&expected), json(&validate_bytes(&d.build())));
    }
}

#[test]
fn glossary_strict_edge_does_not_choose_a_profile_in_either_order() {
    for reverse in [false, true] {
        let mut d = fixture("<w:p/>", &[TRANSITIONAL], false);
        d.add_with_override("word/glossary.xml", CT_GLOSSARY, format!(r#"<w:glossaryDocument {NS}><w:docParts/></w:glossaryDocument>"#));
        d.set("word/strict.xml", format!(r#"<w:glossaryDocument xmlns:w="{STRICT_W}"/>"#));
        let a = relationship("g", TRANSITIONAL, "glossaryDocument", "glossary.xml");
        let b = relationship("s", STRICT, "glossaryDocument", "strict.xml");
        d.set_doc_rels(&(if reverse { b + &a } else { a + &b }));
        let report = validate_bytes(&d.build());
        assert_eq!(report.verdict, Verdict::Pass, "{report:?}");
        assert!(report.coverage_notes.is_empty(), "{report:?}");
    }
}

#[test]
fn strict_owner_markup_is_gated_without_relationship_profile_inference() {
    for glossary in [false, true] {
        let mut d = fixture(DANGLING, &[TRANSITIONAL], false);
        if glossary {
            d.add_with_override("word/glossary.xml", CT_GLOSSARY, format!(r#"<w:glossaryDocument xmlns:w="{STRICT_W}"/>"#));
            // Content-type fallback: no relationship profile is available to choose.
        } else {
            d.set_document_raw(&format!(r#"<w:document xmlns:w="{STRICT_W}"/>"#));
        }
        let report = validate_bytes(&d.build());
        no_document_verdict(&report);
        assert!(report.coverage_notes.iter().any(|n| n.contains("Strict") && n.contains("has root")), "{report:?}");
    }
}
