#![allow(dead_code)]
mod common;
use common::{CT_GLOSSARY, Doc, NS};
use wordcraft_validate::{Report, validate_bytes};
const A: &str =
    r#"<Relationship Id="glossA" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/glossaryDocument" Target="glossA.xml"/>"#;
const B: &str =
    r#"<Relationship Id="glossB" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/glossaryDocument" Target="glossB.xml"/>"#;
fn fixture(typed: bool) -> Doc {
    let mut d = Doc::minimal();
    let a = format!(
        r#"<w:glossaryDocument {NS}><w:docParts><w:docPart><w:docPartBody><w:p><w:r><w:footnoteReference w:id="42"/></w:r></w:p></w:docPartBody></w:docPart></w:docParts></w:glossaryDocument>"#
    );
    let b = format!(r#"<w:glossaryDocument {NS}><w:docParts/></w:glossaryDocument>"#);
    if typed {
        d.add_with_override("word/glossA.xml", CT_GLOSSARY, a);
    } else {
        d.set("word/glossA.xml", a);
    }
    d.set("word/glossB.xml", b);
    d
}
fn has(report: &Report, rule: &str) -> bool {
    report.findings.iter().any(|f| f.rule == rule && f.part.as_deref() == Some("word/glossA.xml"))
}
fn truncated_rels() -> String {
    let dummy: String =
        (0..19_999).map(|i| format!(r#"<Relationship Id="r{i}" Type="urn:custom" Target="https://example.org/" TargetMode="External"/>"#)).collect();
    format!("{A}{dummy}{B}")
}
#[test]
fn incomplete_glossary_selection_does_not_establish_untyped_owner() {
    let mut d = fixture(false);
    d.set_doc_rels(&format!("{A}{B}"));
    let complete = validate_bytes(&d.build());
    assert!(!has(&complete, "REF-NOTE"), "{complete:?}");
    assert!(has(&complete, "CT-MISMATCH"), "{complete:?}");
    d.set_doc_rels(&truncated_rels());
    let partial = validate_bytes(&d.build());
    assert!(!has(&partial, "REF-NOTE"), "{partial:?}");
    assert!(has(&partial, "CT-MISMATCH"), "{partial:?}");
    assert!(partial.coverage_notes.iter().any(|n| n.contains("unknown glossaryDocument selection")), "{partial:?}");
}
#[test]
fn complete_glossary_selection_still_proves_missing_definition() {
    let mut d = fixture(false);
    d.set_doc_rels(A);
    let report = validate_bytes(&d.build());
    assert!(has(&report, "REF-NOTE"), "{report:?}");
    assert!(has(&report, "CT-MISMATCH"), "{report:?}");
}
#[test]
fn absent_glossary_selection_does_not_establish_untyped_owner() {
    let report = validate_bytes(&fixture(false).build());
    assert!(!has(&report, "REF-NOTE"), "{report:?}");
}
#[test]
fn independent_typed_glossary_root_survives_selection_loss() {
    let mut d = fixture(true);
    d.set_doc_rels(&truncated_rels());
    let report = validate_bytes(&d.build());
    assert!(has(&report, "REF-NOTE"), "{report:?}");
}
