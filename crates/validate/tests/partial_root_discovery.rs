#![allow(dead_code, clippy::unwrap_used)]
mod common;
use common::{Doc, document_xml, rels_xml};
use wordcraft_validate::validate_bytes;
#[test]
fn retained_root_with_hidden_competitor_cannot_prove_absence() {
    let mut d = Doc::minimal();
    d.set_document(r#"<w:p><w:r><w:footnoteReference w:id="42"/></w:r></w:p>"#);
    d.set("word/other.xml", document_xml("<w:p/>"));
    let a = r#"<Relationship Id="mainA" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>"#;
    let b = r#"<Relationship Id="mainB" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/other.xml"/>"#;
    d.set("_rels/.rels", rels_xml(&format!("{a}{b}")));
    let complete = validate_bytes(&d.build());

    assert!(!complete.findings.iter().any(|f| f.rule == "REF-NOTE"));
    let dummy: String =
        (0..19_999).map(|i| format!(r#"<Relationship Id="r{i}" Type="urn:custom" Target="https://example.org/" TargetMode="External"/>"#)).collect();
    d.set("_rels/.rels", rels_xml(&format!("{a}{dummy}{b}")));
    let truncated = validate_bytes(&d.build());

    assert_eq!(truncated.verdict, wordcraft_validate::Verdict::Incomplete, "{truncated:?}");
    assert!(!truncated.findings.iter().any(|f| f.rule == "REF-NOTE"), "{truncated:?}");
}
