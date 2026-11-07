//! Integration tests for `wordcraft-validate`, one per contractual rule id. Every negative fixture
//! asserts the rule id **and** the part it is reported against (F11), so a test can never pass on
//! the right rule reported against the wrong part.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::{Path, PathBuf};

use common::{
    CT_CHART, CT_COMMENTS, CT_COMMENTS_EXT, CT_COMMENTS_IDS, CT_CUSTOMXML_PROPS, CT_ENDNOTES, CT_FOOTER, CT_FOOTNOTES, CT_GLOSSARY, CT_HEADER,
    CT_MAIN, CT_NUMBERING, CT_PEOPLE, CT_SETTINGS, CT_STYLES, CT_TEMPLATE_MAIN, CT_WEB_SETTINGS, Doc, add_header, claim_entry_method,
    content_types_xml, corrupt_entry_data, ns, rels_xml, replace_all, rich_content_types, utf16_le_with_bom,
};
use wordcraft_validate::{Finding, Report, Severity, Verdict, validate_bytes, validate_path};

const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const MC_NS: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
const W14_NS: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
const W15_NS: &str = "http://schemas.microsoft.com/office/word/2012/wordml";
const W16CID_NS: &str = "http://schemas.microsoft.com/office/word/2016/wordml/cid";
const DS_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/customXml";

/// Every rule id a minimal, fully checkable package runs, in the documented order.
/// `COMMENTEX-LINK` is absent: it only runs when commentsExtended.xml exists.
const ALL_CHECKS: &[&str] = &[
    "PKG-NOT-ZIP",
    "PKG-DUP-ENTRY",
    "PKG-CASE-COLLISION",
    "PKG-NO-CONTENT-TYPES",
    "PKG-NO-MAIN",
    "CT-UNDECLARED",
    "CT-MAIN-TYPE",
    "CT-DUP-MAIN",
    "CT-DUP-DEFAULT",
    "CT-DUP-OVERRIDE",
    "XML-MALFORMED",
    "XML-UNDECLARED-PREFIX",
    "XML-INVALID-CHAR",
    "REL-DUP-ID",
    "REL-DANGLING",
    "REL-TYPED",
    "REL-REF-MISSING",
    "CT-MISMATCH",
    "ID-COMMENT-DUP",
    "ID-COMMENT-REF",
    "ID-DOCPR-DUP",
    "ID-BOOKMARK-DUP",
    "BOOKMARK-UNPAIRED",
    "ID-REVISION-DUP",
    "NOTE-DUP",
    "REF-NOTE",
    "REF-HEADER",
    "REF-STYLE",
    "NUM-DANGLING",
    "FLD-UNBALANCED",
    "STRUCT-TC-NO-P",
    "STRUCT-DELTEXT",
    "STRUCT-MOVE-UNPAIRED",
    "CUSTOMXML-SCHEMA",
    "MEDIA-MAGIC",
];

fn validate(doc: &Doc) -> Report {
    validate_bytes(&doc.build())
}

fn has(report: &Report, rule: &str) -> bool {
    report.findings.iter().any(|f| f.rule == rule)
}

/// The F11 negative-fixture helper: assert the rule fired **at `part`** (`None` = package-level).
fn assert_finding<'a>(report: &'a Report, rule: &str, part: Option<&str>) -> &'a Finding {
    report
        .findings
        .iter()
        .find(|f| f.rule == rule && f.part.as_deref() == part)
        .unwrap_or_else(|| panic!("expected rule {rule} at part {part:?}; findings were {:#?}", report.findings))
}

fn assert_clean(report: &Report) {
    let bad: Vec<_> = report.findings.iter().filter(|f| matches!(f.severity, Severity::Corrupt | Severity::Error)).collect();
    assert!(bad.is_empty(), "unexpected Corrupt/Error findings: {bad:?}");
}

#[test]
fn minimal_document_passes_with_every_check_run() {
    let report = validate(&Doc::minimal());
    assert_eq!(report.verdict, Verdict::Pass);
    assert!(report.findings.is_empty(), "{:?}", report.findings);
    assert_eq!(report.checks_run, ALL_CHECKS);
}

#[test]
fn rich_document_passes() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", rich_content_types());
    add_header(&mut d);
    d.set(
        "word/footnotes.xml",
        format!(r#"<?xml version="1.0"?><w:footnotes xmlns:w="{W_NS}"><w:footnote w:id="1"><w:p><w:r><w:t>Note</w:t></w:r></w:p></w:footnote></w:footnotes>"#),
    );
    d.set(
        "word/comments.xml",
        format!(r#"<?xml version="1.0"?><w:comments xmlns:w="{W_NS}"><w:comment w:id="1" w:author="A"><w:p><w:r><w:t>Hi</w:t></w:r></w:p></w:comment></w:comments>"#),
    );
    d.set(
        "word/styles.xml",
        format!(r#"<?xml version="1.0"?><w:styles xmlns:w="{W_NS}"><w:style w:type="paragraph" w:styleId="Normal"><w:name w:val="Normal"/></w:style></w:styles>"#),
    );
    d.set("word/media/pic.png", b"\x89PNG\r\n\x1a\nsome png bytes".to_vec());
    d.set_doc_rels(concat!(
        r#"<Relationship Id="rIdH" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/>"#,
        r#"<Relationship Id="rIdImg" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/pic.png"/>"#,
        r#"<Relationship Id="rIdF" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes" Target="footnotes.xml"/>"#,
        r#"<Relationship Id="rIdC" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/>"#,
        r#"<Relationship Id="rIdS" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#,
    ));
    d.set_document(concat!(
        r#"<w:p><w:r><w:t>Hello</w:t></w:r></w:p>"#,
        r#"<w:p><w:r><w:footnoteReference w:id="1"/></w:r></w:p>"#,
        r#"<w:p><w:r><w:commentReference w:id="1"/></w:r></w:p>"#,
        r#"<w:p><w:bookmarkStart w:id="1" w:name="bm"/><w:bookmarkEnd w:id="1"/></w:p>"#,
        r#"<w:p><w:pPr><w:pStyle w:val="Normal"/></w:pPr><w:r><w:t>Styled</w:t></w:r></w:p>"#,
        r#"<w:p><w:r><wp:docPr id="1" name="pic"/></w:r></w:p>"#,
        r#"<w:sectPr><w:headerReference w:type="default" r:id="rIdH"/></w:sectPr>"#,
    ));
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass);
    assert_clean(&report);
    assert!(report.findings.is_empty(), "{:?}", report.findings);
}

#[test]
fn pkg_not_zip() {
    let report = validate_bytes(b"definitely not a zip archive");
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "PKG-NOT-ZIP", None);
    assert_eq!(report.checks_run, vec!["PKG-NOT-ZIP"]);
    assert_eq!(report.findings[0].severity, Severity::Corrupt);
}

#[test]
fn pkg_dup_entry() {
    let mut d = Doc::minimal();
    d.set("word/dA.xml", b"<a/>".to_vec());
    d.set("word/dB.xml", b"<b/>".to_vec());
    let zip = replace_all(&d.build(), b"word/dB.xml", b"word/dA.xml");
    let report = validate_bytes(&zip);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "PKG-DUP-ENTRY", None);
}

/// §2 canary: an archive the zip reader accepts but the raw central-directory parser rejects must be
/// INCOMPLETE with the coverage note, not a silent PASS. A prepended stub shifts every stored
/// offset; the zip reader adjusts for it, but the raw EOCD reader reads the central-directory
/// offset literally and so fails to parse. Removing the incomplete marking restores PASS.
#[test]
fn unparseable_central_directory_is_incomplete() {
    let mut bytes = vec![b'X'; 64];
    bytes.extend_from_slice(&Doc::minimal().build());
    let report = validate_bytes(&bytes);
    assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
    assert!(
        report.coverage_notes.iter().any(|n| n.contains("central directory could not be parsed")),
        "the coverage note must say duplicate checking did not run: {:?}",
        report.coverage_notes
    );
}

#[test]
fn pkg_case_collision() {
    let mut d = Doc::minimal();
    d.set("word/media/Image.png", b"\x89PNG".to_vec());
    d.set("word/media/image.png", b"\x89PNG".to_vec());
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "PKG-CASE-COLLISION", None);
}

#[test]
fn pkg_no_content_types() {
    let mut d = Doc::minimal();
    d.remove("[Content_Types].xml");
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "PKG-NO-CONTENT-TYPES", None);
    assert!(!report.checks_run.contains(&"CT-UNDECLARED"), "content-type checks must not run");
}

#[test]
fn pkg_no_main() {
    let mut d = Doc::minimal();
    d.remove("_rels/.rels");
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "PKG-NO-MAIN", None);
}

#[test]
fn ct_undeclared() {
    let mut d = Doc::minimal();
    d.set("word/extra.bin", b"opaque".to_vec());
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "CT-UNDECLARED", Some("word/extra.bin"));
}

#[test]
fn ct_main_type() {
    let mut d = Doc::minimal();
    let ct = String::from_utf8(content_types_xml(&[], &[])).unwrap().replace(CT_MAIN, "application/xml");
    d.set("[Content_Types].xml", ct);
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "CT-MAIN-TYPE", Some("word/document.xml"));
}

#[test]
fn ct_dup_default() {
    let mut d = Doc::minimal();
    let ct = String::from_utf8(content_types_xml(&[("png", "image/png")], &[]))
        .unwrap()
        .replace("</Types>", r#"<Default Extension="png" ContentType="image/png"/></Types>"#);
    d.set("[Content_Types].xml", ct);
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    let f = assert_finding(&report, "CT-DUP-DEFAULT", Some("[Content_Types].xml"));
    assert_eq!(f.severity, Severity::Error);
    assert_eq!(f.evidence, wordcraft_validate::Evidence::ByConstruction);
}

#[test]
fn ct_dup_override() {
    let mut d = Doc::minimal();
    let extra = format!(r#"<Override PartName="/word/document.xml" ContentType="{CT_MAIN}"/>"#);
    let ct = String::from_utf8(content_types_xml(&[], &[])).unwrap().replace("</Types>", &format!("{extra}</Types>"));
    d.set("[Content_Types].xml", ct);
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "CT-DUP-OVERRIDE", Some("[Content_Types].xml"));
}

#[test]
fn xml_malformed_in_main_is_corrupt() {
    let mut d = Doc::minimal();
    d.set_document_raw(&format!(r#"<?xml version="1.0"?><w:document xmlns:w="{W_NS}"><w:body></w:document>"#));
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    let f = assert_finding(&report, "XML-MALFORMED", Some("word/document.xml"));
    assert_eq!(f.severity, Severity::Corrupt);
}

#[test]
fn xml_empty_main_part_is_malformed() {
    let mut d = Doc::minimal();
    d.set_document_raw("");
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    let f = assert_finding(&report, "XML-MALFORMED", Some("word/document.xml"));
    assert_eq!(f.severity, Severity::Corrupt);
    assert!(f.message.contains("document element"), "{}", f.message);
}

#[test]
fn xml_two_roots_is_malformed() {
    let mut d = Doc::minimal();
    d.set_document_raw(&format!(r#"<?xml version="1.0"?><w:document xmlns:w="{W_NS}"/><w:document xmlns:w="{W_NS}"/>"#));
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    let f = assert_finding(&report, "XML-MALFORMED", Some("word/document.xml"));
    assert!(f.message.contains("document element"), "{}", f.message);
}

#[test]
fn xml_duplicate_attribute_is_malformed() {
    let mut d = Doc::minimal();
    d.set_document_raw(&format!(r#"<?xml version="1.0"?><w:document xmlns:w="{W_NS}" w:dup="1" w:dup="2"><w:body/></w:document>"#));
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "XML-MALFORMED", Some("word/document.xml"));
}

#[test]
fn xml_unclosed_attribute_quote_is_malformed() {
    let mut d = Doc::minimal();
    d.set_document_raw(&format!(r#"<?xml version="1.0"?><w:document xmlns:w="{W_NS}"><w:p w:val="unclosed/></w:document>"#));
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "XML-MALFORMED", Some("word/document.xml"));
}

#[test]
fn xml_malformed_unreferenced_is_a_warning_and_verdict_stays_pass() {
    let mut d = Doc::minimal();
    d.set("word/junk.xml", b"<junk>".to_vec());
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass);
    let f = assert_finding(&report, "XML-MALFORMED", Some("word/junk.xml"));
    assert_eq!(f.severity, Severity::Warning);
    assert_eq!(f.evidence, wordcraft_validate::Evidence::Observed);
}

#[test]
fn xml_undeclared_prefix() {
    let mut d = Doc::minimal();
    d.set_document_raw(&format!(r#"<?xml version="1.0"?><w:document {}><w:body><foo:bar/></w:body></w:document>"#, ns(true, false)));
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    let f = assert_finding(&report, "XML-UNDECLARED-PREFIX", Some("word/document.xml"));
    assert_eq!(f.severity, Severity::Corrupt);
}

#[test]
fn undeclared_mc_ignorable_prefix() {
    let mut d = Doc::minimal();
    d.set_document_raw(&format!(
        r#"<?xml version="1.0"?><w:document xmlns:w="{W_NS}" xmlns:mc="{MC_NS}"><w:body><w:p mc:Ignorable="w14"/></w:body></w:document>"#
    ));
    let report = validate(&d);
    assert_finding(&report, "XML-UNDECLARED-PREFIX", Some("word/document.xml"));
}

#[test]
fn xml_invalid_char_literal() {
    let mut d = Doc::minimal();
    let body = format!("<w:p><w:r><w:t>bad{}\u{000B}char</w:t></w:r></w:p>", "");
    d.set_document(&body);
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    let f = assert_finding(&report, "XML-INVALID-CHAR", Some("word/document.xml"));
    assert_eq!(f.severity, Severity::Corrupt);
}

#[test]
fn xml_invalid_char_reference() {
    let mut d = Doc::minimal();
    d.set_document(r#"<w:p><w:r><w:t>bad&#xB;</w:t></w:r></w:p>"#);
    let report = validate(&d);
    assert_finding(&report, "XML-INVALID-CHAR", Some("word/document.xml"));
}

#[test]
fn utf16_document_part_is_valid() {
    let mut d = Doc::minimal();
    let xml = format!(r#"<?xml version="1.0" encoding="UTF-16"?><w:document xmlns:w="{W_NS}"><w:body><w:p/></w:body></w:document>"#);
    d.set("word/document.xml", utf16_le_with_bom(&xml));
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass, "findings: {:?}", report.findings);
    assert!(!has(&report, "XML-INVALID-CHAR"));
    assert!(!has(&report, "XML-MALFORMED"), "{:?}", report.findings);
}

#[test]
fn cdata_with_character_reference_is_not_flagged() {
    let mut d = Doc::minimal();
    d.set_document(r#"<w:p><w:r><w:t><![CDATA[&#x0;]]></w:t></w:r></w:p>"#);
    let report = validate(&d);
    assert!(!has(&report, "XML-INVALID-CHAR"), "{:?}", report.findings);
}

#[test]
fn comment_with_character_reference_is_not_flagged() {
    let mut d = Doc::minimal();
    d.set_document(r#"<w:p><!--&#x0;--></w:p>"#);
    let report = validate(&d);
    assert!(!has(&report, "XML-INVALID-CHAR"), "{:?}", report.findings);
}

#[test]
fn xml_allowed_control_chars_do_not_fire() {
    let mut d = Doc::minimal();
    d.set_document("<w:p><w:r><w:t>tab\there\nand&#x9;ref</w:t></w:r></w:p>");
    let report = validate(&d);
    assert!(!has(&report, "XML-INVALID-CHAR"), "{:?}", report.findings);
}

#[test]
fn rel_dup_id() {
    let mut d = Doc::minimal();
    d.set_doc_rels(concat!(
        r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://one.example/" TargetMode="External"/>"#,
        r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://two.example/" TargetMode="External"/>"#,
    ));
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "REL-DUP-ID", Some("word/_rels/document.xml.rels"));
    assert!(!has(&report, "REL-DANGLING"), "external targets never dangle");
}

#[test]
fn the_same_id_in_two_rels_parts_is_not_a_duplicate() {
    let mut d = Doc::minimal();
    d.set_doc_rels(r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://one.example/" TargetMode="External"/>"#);
    let report = validate(&d);
    assert!(!has(&report, "REL-DUP-ID"), "rId1 in another .rels part is legal");
}

#[test]
fn rel_dangling() {
    let mut d = Doc::minimal();
    d.set_doc_rels(
        r#"<Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/missing.png"/>"#,
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    let f = assert_finding(&report, "REL-DANGLING", Some("word/_rels/document.xml.rels"));
    assert!(f.message.contains("missing"), "message should say the content is missing: {}", f.message);
}

#[test]
fn external_relationship_does_not_dangle() {
    let mut d = Doc::minimal();
    d.set_doc_rels(r#"<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com/" TargetMode="External"/>"#);
    let report = validate(&d);
    assert!(!has(&report, "REL-DANGLING"));
}

#[test]
fn percent_encoded_target_resolves() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[("png", "image/png")], &[]));
    d.set("word/media/my image.png", b"\x89PNG\r\n\x1a\npng body".to_vec());
    d.set_doc_rels(
        r#"<Relationship Id="rIdI" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/my%20image.png"/>"#,
    );
    let report = validate(&d);
    assert!(!has(&report, "REL-DANGLING"), "{:?}", report.findings);
}

#[test]
fn target_with_query_and_fragment_resolves() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[("png", "image/png")], &[]));
    d.set("word/media/pic.png", b"\x89PNG\r\n\x1a\npng body".to_vec());
    d.set_doc_rels(
        r#"<Relationship Id="rIdI" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/pic.png?x=1#frag"/>"#,
    );
    let report = validate(&d);
    assert!(!has(&report, "REL-DANGLING"), "{:?}", report.findings);
}

#[test]
fn rel_typed_header() {
    let mut d = Doc::minimal();
    add_header(&mut d);
    d.set_doc_rels(
        r#"<Relationship Id="rIdH" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/>"#,
    );
    d.set_document(r#"<w:p/><w:sectPr><w:headerReference w:type="default" r:id="rIdH"/></w:sectPr>"#);
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "REL-TYPED", Some("word/_rels/document.xml.rels"));
    assert!(
        !has(&report, "CT-MISMATCH"),
        "a main-owned header relationship stays REL-TYPED's alone; CT-MISMATCH must not double-report it: {:?}",
        report.findings
    );
}

#[test]
fn rel_typed_image() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/word/media/pic.bin", "application/octet-stream")]));
    d.set("word/media/pic.bin", b"\x89PNG not image typed".to_vec());
    d.set_doc_rels(
        r#"<Relationship Id="rIdI" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/pic.bin"/>"#,
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "REL-TYPED", Some("word/_rels/document.xml.rels"));
}

#[test]
fn rel_ref_missing() {
    let mut d = Doc::minimal();
    d.set_document(r#"<w:p><w:hyperlink r:id="rId9"><w:r><w:t>x</w:t></w:r></w:hyperlink></w:p>"#);
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    let f = assert_finding(&report, "REL-REF-MISSING", Some("word/document.xml"));
    assert_eq!(f.evidence, wordcraft_validate::Evidence::Inferred);
}

#[test]
fn rel_ref_present_does_not_fire() {
    let mut d = Doc::minimal();
    d.set_doc_rels(r#"<Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com/" TargetMode="External"/>"#);
    d.set_document(r#"<w:p><w:hyperlink r:id="rId9"><w:r><w:t>x</w:t></w:r></w:hyperlink></w:p>"#);
    let report = validate(&d);
    assert!(!has(&report, "REL-REF-MISSING"), "{:?}", report.findings);
}

#[test]
fn ct_mismatch() {
    let mut d = Doc::minimal();
    d.add_with_override("word/comments.xml", CT_STYLES, comments(r#"<w:comment w:id="1"><w:p/></w:comment>"#));
    d.set_doc_rels(
        r#"<Relationship Id="rIdC" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/>"#,
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "CT-MISMATCH", Some("word/comments.xml"));
}

#[test]
fn correct_fixed_type_does_not_mismatch() {
    let mut d = Doc::minimal();
    d.add_with_override("word/comments.xml", CT_COMMENTS, comments(r#"<w:comment w:id="1"><w:p/></w:comment>"#));
    d.set_doc_rels(
        r#"<Relationship Id="rIdC" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/>"#,
    );
    let report = validate(&d);
    assert!(!has(&report, "CT-MISMATCH"), "{:?}", report.findings);
}

fn comments(inner: &str) -> Vec<u8> {
    format!(r#"<?xml version="1.0"?><w:comments xmlns:w="{W_NS}">{inner}</w:comments>"#).into_bytes()
}

/// The distilled corpus defect: a glossary part declared with the main content type behind a
/// correct `glossaryDocument` relationship. CT-MISMATCH fires at the glossary part (the pairing is
/// the one oracle-observed row) and CT-DUP-MAIN fires at the same part, while the real main part
/// is not blamed.
#[test]
fn ct_mismatch_glossary_part_typed_as_main_fires_dup_main() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/word/glossary.xml", CT_MAIN)]));
    d.set_doc_rels(
        r#"<Relationship Id="rG" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/glossaryDocument" Target="glossary.xml"/>"#,
    );
    d.set("word/glossary.xml", format!(r#"<?xml version="1.0"?><w:glossaryDocument xmlns:w="{W_NS}"><w:docParts/></w:glossaryDocument>"#));
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    let f = assert_finding(&report, "CT-MISMATCH", Some("word/glossary.xml"));
    assert_eq!(f.evidence, wordcraft_validate::Evidence::Observed, "the glossary pairing carries the Word-web observation");
    assert_finding(&report, "CT-DUP-MAIN", Some("word/glossary.xml"));
    assert!(
        !report.findings.iter().any(|f| f.rule == "CT-DUP-MAIN" && f.part.as_deref() == Some("word/document.xml")),
        "the part the officeDocument relationship reaches must not be reported: {:?}",
        report.findings
    );
}

/// A newly covered kind, first of three: a chart part declared `image/png` behind a correct chart
/// relationship. CT-MISMATCH fires at the chart part.
#[test]
fn ct_mismatch_chart_typed_as_image() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[("png", "image/png")], &[("/word/charts/chart1.xml", "image/png")]));
    d.set_doc_rels(
        r#"<Relationship Id="rCh" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="charts/chart1.xml"/>"#,
    );
    d.set("word/charts/chart1.xml", r#"<?xml version="1.0"?><c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"/>"#);
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "CT-MISMATCH", Some("word/charts/chart1.xml"));
}

/// A newly covered kind, second of three: a web-settings part declared with the settings type
/// behind a correct `webSettings` relationship. CT-MISMATCH fires at the web-settings part.
#[test]
fn ct_mismatch_websettings_typed_as_settings() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/word/webSettings.xml", CT_SETTINGS)]));
    d.set_doc_rels(
        r#"<Relationship Id="rW" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/webSettings" Target="webSettings.xml"/>"#,
    );
    d.set("word/webSettings.xml", format!(r#"<?xml version="1.0"?><w:webSettings xmlns:w="{W_NS}"/>"#));
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "CT-MISMATCH", Some("word/webSettings.xml"));
}

/// A newly covered kind, third of three: the commentsIds part declared with the commentsExtended
/// type (a sibling swap) behind a correct `commentsIds` relationship. CT-MISMATCH fires at the
/// commentsIds part, carrying the Word-web observation.
#[test]
fn ct_mismatch_comments_ids_typed_as_comments_extended() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/word/commentsIds.xml", CT_COMMENTS_EXT)]));
    d.set_doc_rels(r#"<Relationship Id="rCI" Type="http://schemas.microsoft.com/office/2011/relationships/commentsIds" Target="commentsIds.xml"/>"#);
    d.set("word/commentsIds.xml", format!(r#"<?xml version="1.0"?><w16cid:commentsIds xmlns:w16cid="{W16CID_NS}"/>"#));
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    let f = assert_finding(&report, "CT-MISMATCH", Some("word/commentsIds.xml"));
    assert_eq!(f.evidence, wordcraft_validate::Evidence::Observed, "the commentsIds pairing carries the Word-web observation");
}

/// The commentsExtended part declared with another catalogue type behind a correct
/// `commentsExtended` relationship. CT-MISMATCH fires at the commentsExtended part, carrying the
/// Word-web observation.
#[test]
fn ct_mismatch_comments_extended_typed_as_comments() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/word/commentsExtended.xml", CT_COMMENTS)]));
    d.set_doc_rels(
        r#"<Relationship Id="rCE" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/commentsExtended" Target="commentsExtended.xml"/>"#,
    );
    d.set("word/commentsExtended.xml", format!(r#"<?xml version="1.0"?><w15:commentsEx xmlns:w15="{W15_NS}"/>"#));
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    let f = assert_finding(&report, "CT-MISMATCH", Some("word/commentsExtended.xml"));
    assert_eq!(f.evidence, wordcraft_validate::Evidence::Observed, "the commentsExtended pairing carries the Word-web observation");
}

/// The people part declared `application/xml` behind a correct `people` relationship. CT-MISMATCH
/// fires at the people part, carrying the Word-web observation.
#[test]
fn ct_mismatch_people_typed_as_xml() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/word/people.xml", "application/xml")]));
    d.set_doc_rels(r#"<Relationship Id="rP" Type="http://schemas.microsoft.com/office/2011/relationships/people" Target="people.xml"/>"#);
    d.set("word/people.xml", format!(r#"<?xml version="1.0"?><w15:people xmlns:w15="{W15_NS}"/>"#));
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    let f = assert_finding(&report, "CT-MISMATCH", Some("word/people.xml"));
    assert_eq!(f.evidence, wordcraft_validate::Evidence::Observed, "the people pairing carries the Word-web observation");
}

/// A second main-typed part that no relationship reaches: CT-DUP-MAIN fires at it, while
/// CT-MISMATCH cannot see it at all (no fixed-type relationship points at it).
#[test]
fn ct_dup_main_unreferenced_extra_part() {
    let mut d = Doc::minimal();
    d.add_with_override("word/extra.xml", CT_MAIN, format!(r#"<?xml version="1.0"?><w:document xmlns:w="{W_NS}"><w:body/></w:document>"#));
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "CT-DUP-MAIN", Some("word/extra.xml"));
    assert!(!has(&report, "CT-MISMATCH"), "no relationship reaches the extra part, so the pairing check cannot flag it: {:?}", report.findings);
}

/// The broad positive control for the pairing table: every newly covered pairing, correctly typed,
/// produces neither CT-MISMATCH nor CT-DUP-MAIN.
#[test]
fn correct_catalogue_pairings_do_not_fire() {
    let mut d = Doc::minimal();
    d.set(
        "[Content_Types].xml",
        content_types_xml(
            &[],
            &[
                ("/word/glossary.xml", CT_GLOSSARY),
                ("/word/charts/chart1.xml", CT_CHART),
                ("/word/webSettings.xml", CT_WEB_SETTINGS),
                ("/word/commentsIds.xml", CT_COMMENTS_IDS),
                ("/word/people.xml", CT_PEOPLE),
                ("/word/bibliography.xml", "application/xml"),
                ("/word/customXml/itemProps1.xml", CT_CUSTOMXML_PROPS),
            ],
        ),
    );
    d.set_doc_rels(concat!(
        r#"<Relationship Id="rG" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/glossaryDocument" Target="glossary.xml"/>"#,
        r#"<Relationship Id="rCh" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="charts/chart1.xml"/>"#,
        r#"<Relationship Id="rW" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/webSettings" Target="webSettings.xml"/>"#,
        r#"<Relationship Id="rCI" Type="http://schemas.microsoft.com/office/2011/relationships/commentsIds" Target="commentsIds.xml"/>"#,
        r#"<Relationship Id="rP" Type="http://schemas.microsoft.com/office/2011/relationships/people" Target="people.xml"/>"#,
        r#"<Relationship Id="rB" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/bibliography" Target="bibliography.xml"/>"#,
        r#"<Relationship Id="rCx" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="customXml/item1.xml"/>"#,
    ));
    d.set(
        "word/customXml/_rels/item1.xml.rels",
        rels_xml(
            r#"<Relationship Id="rCp" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXmlProps" Target="itemProps1.xml"/>"#,
        ),
    );
    d.set("word/glossary.xml", format!(r#"<?xml version="1.0"?><w:glossaryDocument xmlns:w="{W_NS}"><w:docParts/></w:glossaryDocument>"#));
    d.set("word/charts/chart1.xml", r#"<?xml version="1.0"?><c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"/>"#);
    d.set("word/webSettings.xml", format!(r#"<?xml version="1.0"?><w:webSettings xmlns:w="{W_NS}"/>"#));
    d.set("word/commentsIds.xml", format!(r#"<?xml version="1.0"?><w16cid:commentsIds xmlns:w16cid="{W16CID_NS}"/>"#));
    d.set("word/people.xml", format!(r#"<?xml version="1.0"?><w15:people xmlns:w15="{W15_NS}"/>"#));
    d.set(
        "word/bibliography.xml",
        r#"<?xml version="1.0"?><b:Sources xmlns:b="http://schemas.openxmlformats.org/officeDocument/2006/bibliography"/>"#,
    );
    d.set("word/customXml/item1.xml", r#"<?xml version="1.0"?><stressRoot xmlns="urn:example:stress"/>"#);
    d.set(
        "word/customXml/itemProps1.xml",
        format!(r#"<?xml version="1.0"?><ds:datastoreItem xmlns:ds="{DS_NS}" ds:itemID="{{S}}"><ds:schemaRefs/></ds:datastoreItem>"#),
    );
    let report = validate(&d);
    assert!(!has(&report, "CT-MISMATCH"), "{:?}", report.findings);
    assert!(!has(&report, "CT-DUP-MAIN"), "{:?}", report.findings);
    assert_clean(&report);
}

/// A header relationship owned by the **glossary document** (the Part 1 catalogue lists the
/// glossary document as a header source, §11.3.9) targets a part typed `application/xml`. The
/// main-owned header/footer path (`REL-TYPED`) never sees it and the pairing table previously had
/// no header row, so this escaped both; CT-MISMATCH now covers a non-main owner's relationships.
#[test]
fn ct_mismatch_glossary_owned_header_typed_as_xml() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/word/glossary.xml", CT_GLOSSARY), ("/word/headerGloss.xml", "application/xml")]));
    d.set_doc_rels(
        r#"<Relationship Id="rG" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/glossaryDocument" Target="glossary.xml"/>"#,
    );
    d.set(
        "word/_rels/glossary.xml.rels",
        rels_xml(
            r#"<Relationship Id="rH" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="headerGloss.xml"/>"#,
        ),
    );
    d.set("word/glossary.xml", format!(r#"<?xml version="1.0"?><w:glossaryDocument xmlns:w="{W_NS}"><w:docParts/></w:glossaryDocument>"#));
    d.set("word/headerGloss.xml", format!(r#"<?xml version="1.0"?><w:hdr xmlns:w="{W_NS}"><w:p/></w:hdr>"#));
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    let f = assert_finding(&report, "CT-MISMATCH", Some("word/headerGloss.xml"));
    assert_eq!(f.evidence, wordcraft_validate::Evidence::MsDocumented, "the header row is the Part 1 catalogue, not an oracle observation");
}

/// A footer relationship owned by the glossary document targets a part typed with the settings
/// type. CT-MISMATCH covers the glossary-owned footer pairing and reports at the footer part.
#[test]
fn ct_mismatch_glossary_owned_footer_typed_as_settings() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/word/glossary.xml", CT_GLOSSARY), ("/word/footerGloss.xml", CT_SETTINGS)]));
    d.set_doc_rels(
        r#"<Relationship Id="rG" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/glossaryDocument" Target="glossary.xml"/>"#,
    );
    d.set(
        "word/_rels/glossary.xml.rels",
        rels_xml(
            r#"<Relationship Id="rF" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footerGloss.xml"/>"#,
        ),
    );
    d.set("word/glossary.xml", format!(r#"<?xml version="1.0"?><w:glossaryDocument xmlns:w="{W_NS}"><w:docParts/></w:glossaryDocument>"#));
    d.set("word/footerGloss.xml", format!(r#"<?xml version="1.0"?><w:ftr xmlns:w="{W_NS}"><w:p/></w:ftr>"#));
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "CT-MISMATCH", Some("word/footerGloss.xml"));
}

/// The positive control for the new owner scope: a glossary-owned header **and** footer, each
/// correctly typed, produce no CT-MISMATCH (and the package stays clean).
#[test]
fn correct_glossary_owned_header_and_footer_do_not_mismatch() {
    let mut d = Doc::minimal();
    d.set(
        "[Content_Types].xml",
        content_types_xml(&[], &[("/word/glossary.xml", CT_GLOSSARY), ("/word/headerGloss.xml", CT_HEADER), ("/word/footerGloss.xml", CT_FOOTER)]),
    );
    d.set_doc_rels(
        r#"<Relationship Id="rG" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/glossaryDocument" Target="glossary.xml"/>"#,
    );
    d.set(
        "word/_rels/glossary.xml.rels",
        rels_xml(concat!(
            r#"<Relationship Id="rH" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="headerGloss.xml"/>"#,
            r#"<Relationship Id="rF" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footerGloss.xml"/>"#,
        )),
    );
    d.set("word/glossary.xml", format!(r#"<?xml version="1.0"?><w:glossaryDocument xmlns:w="{W_NS}"><w:docParts/></w:glossaryDocument>"#));
    d.set("word/headerGloss.xml", format!(r#"<?xml version="1.0"?><w:hdr xmlns:w="{W_NS}"><w:p/></w:hdr>"#));
    d.set("word/footerGloss.xml", format!(r#"<?xml version="1.0"?><w:ftr xmlns:w="{W_NS}"><w:p/></w:ftr>"#));
    let report = validate(&d);
    assert!(!has(&report, "CT-MISMATCH"), "{:?}", report.findings);
    assert_clean(&report);
}

/// CT-DUP-MAIN's template branch: the extra main-typed part carries the **template** main variant
/// while the reached part carries the ordinary main type. CT-DUP-MAIN fires at the extra part, and
/// only there.
#[test]
fn ct_dup_main_template_typed_extra_part() {
    let mut d = Doc::minimal();
    d.add_with_override("word/extra.xml", CT_TEMPLATE_MAIN, format!(r#"<?xml version="1.0"?><w:document xmlns:w="{W_NS}"><w:body/></w:document>"#));
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    let dups: Vec<&Finding> = report.findings.iter().filter(|f| f.rule == "CT-DUP-MAIN").collect();
    assert_eq!(dups.len(), 1, "exactly one duplicate-main finding: {:?}", report.findings);
    assert_eq!(dups[0].part.as_deref(), Some("word/extra.xml"));
}

/// The mirror of the template branch: the reached part carries the template main type and the
/// extra part the ordinary main type. STILL exactly one CT-DUP-MAIN, at the extra part — the
/// template variant is one of the package's main types, not a second kind. (CT-MAIN-TYPE is a
/// separate, expected finding at the template-typed reached part.)
#[test]
fn ct_dup_main_template_typed_reached_part() {
    let mut d = Doc::minimal();
    d.add_with_override("word/extra.xml", CT_MAIN, format!(r#"<?xml version="1.0"?><w:document xmlns:w="{W_NS}"><w:body/></w:document>"#));
    let retagged = String::from_utf8(d.get("[Content_Types].xml").to_vec()).unwrap().replace(
        &format!(r#"<Override PartName="/word/document.xml" ContentType="{CT_MAIN}"/>"#),
        &format!(r#"<Override PartName="/word/document.xml" ContentType="{CT_TEMPLATE_MAIN}"/>"#),
    );
    d.set("[Content_Types].xml", retagged);
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    let dups: Vec<&Finding> = report.findings.iter().filter(|f| f.rule == "CT-DUP-MAIN").collect();
    assert_eq!(dups.len(), 1, "exactly one duplicate-main finding, at the extra part: {:?}", report.findings);
    assert_eq!(dups[0].part.as_deref(), Some("word/extra.xml"));
}

#[test]
fn renamed_comments_part_is_found() {
    let mut d = Doc::minimal();
    d.add_with_override(
        "word/comments2.xml",
        CT_COMMENTS,
        comments(r#"<w:comment w:id="1"><w:p/></w:comment><w:comment w:id="1"><w:p/></w:comment>"#),
    );
    d.set_doc_rels(
        r#"<Relationship Id="rIdC" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments2.xml"/>"#,
    );
    d.set_document(r#"<w:p><w:r><w:commentReference w:id="1"/></w:r></w:p>"#);
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "ID-COMMENT-DUP", Some("word/comments2.xml"));
    assert!(!has(&report, "ID-COMMENT-REF"), "a comment at a renamed part still resolves its reference: {:?}", report.findings);
}

#[test]
fn id_comment_dup() {
    let mut d = Doc::minimal();
    d.add_with_override(
        "word/comments.xml",
        CT_COMMENTS,
        comments(r#"<w:comment w:id="1"><w:p/></w:comment><w:comment w:id="1"><w:p/></w:comment>"#),
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    let f = assert_finding(&report, "ID-COMMENT-DUP", Some("word/comments.xml"));
    assert_eq!(f.evidence, wordcraft_validate::Evidence::Observed);
}

#[test]
fn id_comment_ref() {
    let mut d = Doc::minimal();
    d.add_with_override("word/comments.xml", CT_COMMENTS, comments(r#"<w:comment w:id="1"><w:p/></w:comment>"#));
    d.set_document(r#"<w:p><w:r><w:commentReference w:id="5"/></w:r></w:p>"#);
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "ID-COMMENT-REF", Some("word/document.xml"));
    assert!(
        report.coverage_notes.iter().any(|n| n.contains("never referenced")),
        "an unreferenced comment is a coverage note: {:?}",
        report.coverage_notes
    );
}

#[test]
fn id_docpr_dup_is_a_warning() {
    let mut d = Doc::minimal();
    d.set_document(r#"<w:p><w:r><wp:docPr id="1" name="a"/></w:r><w:r><wp:docPr id="1" name="b"/></w:r></w:p>"#);
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass);
    let f = assert_finding(&report, "ID-DOCPR-DUP", Some("word/document.xml"));
    assert_eq!(f.severity, Severity::Warning);
}

#[test]
fn id_bookmark_dup_is_a_warning() {
    let mut d = Doc::minimal();
    d.set_document(r#"<w:p><w:bookmarkStart w:id="1" w:name="a"/><w:bookmarkEnd w:id="1"/><w:bookmarkStart w:id="1" w:name="b"/></w:p>"#);
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass);
    let f = assert_finding(&report, "ID-BOOKMARK-DUP", Some("word/document.xml"));
    assert_eq!(f.severity, Severity::Warning);
}

#[test]
fn a_bookmark_start_end_pair_is_not_a_duplicate() {
    let mut d = Doc::minimal();
    d.set_document(r#"<w:p><w:bookmarkStart w:id="1" w:name="a"/><w:bookmarkEnd w:id="1"/></w:p>"#);
    let report = validate(&d);
    assert!(!has(&report, "ID-BOOKMARK-DUP"));
    assert!(!has(&report, "BOOKMARK-UNPAIRED"));
}

#[test]
fn bookmark_unpaired_in_comments() {
    let mut d = Doc::minimal();
    d.add_with_override(
        "word/comments.xml",
        CT_COMMENTS,
        comments(r#"<w:comment w:id="1"><w:p><w:bookmarkStart w:id="7" w:name="bm"/></w:p></w:comment>"#),
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass);
    let f = assert_finding(&report, "BOOKMARK-UNPAIRED", Some("word/comments.xml"));
    assert_eq!(f.severity, Severity::Warning);
}

#[test]
fn bookmark_paired_in_comments_does_not_fire() {
    let mut d = Doc::minimal();
    d.add_with_override(
        "word/comments.xml",
        CT_COMMENTS,
        comments(r#"<w:comment w:id="1"><w:p><w:bookmarkStart w:id="7" w:name="bm"/><w:bookmarkEnd w:id="7"/></w:p></w:comment>"#),
    );
    let report = validate(&d);
    assert!(!has(&report, "BOOKMARK-UNPAIRED"), "{:?}", report.findings);
}

#[test]
fn id_revision_dup_is_a_warning() {
    let mut d = Doc::minimal();
    d.set_document(r#"<w:p><w:ins w:id="1"><w:r><w:t>a</w:t></w:r></w:ins><w:ins w:id="1"><w:r><w:t>b</w:t></w:r></w:ins></w:p>"#);
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass);
    let f = assert_finding(&report, "ID-REVISION-DUP", Some("word/document.xml"));
    assert_eq!(f.severity, Severity::Warning);
}

#[test]
fn note_dup() {
    let mut d = Doc::minimal();
    d.add_with_override(
        "word/footnotes.xml",
        CT_FOOTNOTES,
        format!(
            r#"<?xml version="1.0"?><w:footnotes xmlns:w="{W_NS}"><w:footnote w:id="1"><w:p/></w:footnote><w:footnote w:id="1"><w:p/></w:footnote></w:footnotes>"#
        ),
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    let f = assert_finding(&report, "NOTE-DUP", Some("word/footnotes.xml"));
    assert_eq!(f.evidence, wordcraft_validate::Evidence::MsDocumented);
}

#[test]
fn note_dup_endnotes() {
    let mut d = Doc::minimal();
    d.add_with_override(
        "word/endnotes.xml",
        CT_ENDNOTES,
        format!(
            r#"<?xml version="1.0"?><w:endnotes xmlns:w="{W_NS}"><w:endnote w:id="1"><w:p/></w:endnote><w:endnote w:id="1"><w:p/></w:endnote></w:endnotes>"#
        ),
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "NOTE-DUP", Some("word/endnotes.xml"));
}

#[test]
fn ref_note() {
    let mut d = Doc::minimal();
    d.add_with_override(
        "word/footnotes.xml",
        CT_FOOTNOTES,
        format!(r#"<?xml version="1.0"?><w:footnotes xmlns:w="{W_NS}"><w:footnote w:id="1"><w:p/></w:footnote></w:footnotes>"#),
    );
    d.set_document(r#"<w:p><w:r><w:footnoteReference w:id="5"/></w:r></w:p>"#);
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    let f = assert_finding(&report, "REF-NOTE", Some("word/document.xml"));
    assert!(f.message.contains("note body is absent"), "message should state the note body is absent: {}", f.message);
}

#[test]
fn a_matching_footnote_reference_is_fine() {
    let mut d = Doc::minimal();
    d.add_with_override(
        "word/footnotes.xml",
        CT_FOOTNOTES,
        format!(r#"<?xml version="1.0"?><w:footnotes xmlns:w="{W_NS}"><w:footnote w:id="1"><w:p/></w:footnote></w:footnotes>"#),
    );
    d.set_document(r#"<w:p><w:r><w:footnoteReference w:id="1"/></w:r></w:p>"#);
    let report = validate(&d);
    assert!(!has(&report, "REF-NOTE"));
}

#[test]
fn ref_header() {
    let mut d = Doc::minimal();
    d.set_document(r#"<w:p/><w:sectPr><w:headerReference w:type="default" r:id="rIdMissing"/></w:sectPr>"#);
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "REF-HEADER", Some("word/document.xml"));
}

#[test]
fn header_reference_to_an_image_relationship_fires() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[("png", "image/png")], &[]));
    d.set("word/media/pic.png", b"\x89PNG\r\n\x1a\npng body".to_vec());
    d.set_doc_rels(
        r#"<Relationship Id="rIdImg" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/pic.png"/>"#,
    );
    d.set_document(r#"<w:p/><w:sectPr><w:headerReference w:type="default" r:id="rIdImg"/></w:sectPr>"#);
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "REF-HEADER", Some("word/document.xml"));
    assert!(!has(&report, "REL-TYPED"), "the image relationship is itself well-typed: {:?}", report.findings);
}

#[test]
fn correct_header_reference_does_not_fire() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", rich_content_types());
    add_header(&mut d);
    d.set_doc_rels(
        r#"<Relationship Id="rIdH" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/>"#,
    );
    d.set_document(r#"<w:p/><w:sectPr><w:headerReference w:type="default" r:id="rIdH"/></w:sectPr>"#);
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass, "findings: {:?}", report.findings);
    assert!(!has(&report, "REF-HEADER"), "{:?}", report.findings);
}

#[test]
fn ref_style_is_a_warning() {
    let mut d = Doc::minimal();
    d.add_with_override(
        "word/styles.xml",
        CT_STYLES,
        format!(r#"<?xml version="1.0"?><w:styles xmlns:w="{W_NS}"><w:style w:type="paragraph" w:styleId="Normal"><w:name w:val="Normal"/></w:style></w:styles>"#),
    );
    d.set_doc_rels(
        r#"<Relationship Id="rS" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#,
    );
    d.set_document(r#"<w:p><w:pPr><w:pStyle w:val="Missing"/></w:pPr><w:r><w:t>x</w:t></w:r></w:p>"#);
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass);
    let f = assert_finding(&report, "REF-STYLE", Some("word/document.xml"));
    assert_eq!(f.severity, Severity::Warning);
}

#[test]
fn num_dangling() {
    let mut d = Doc::minimal();
    d.add_with_override(
        "word/numbering.xml",
        CT_NUMBERING,
        format!(
            r#"<?xml version="1.0"?><w:numbering xmlns:w="{W_NS}"><w:abstractNum w:abstractNumId="0"/><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#
        ),
    );
    d.set_doc_rels(
        r#"<Relationship Id="rN" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/>"#,
    );
    d.set_document(r#"<w:p><w:pPr><w:numPr><w:numId w:val="5"/></w:numPr></w:pPr><w:r><w:t>x</w:t></w:r></w:p>"#);
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass);
    assert_finding(&report, "NUM-DANGLING", Some("word/document.xml"));
}

#[test]
fn num_dangling_definition_side() {
    let mut d = Doc::minimal();
    d.add_with_override(
        "word/numbering.xml",
        CT_NUMBERING,
        format!(
            r#"<?xml version="1.0"?><w:numbering xmlns:w="{W_NS}"><w:abstractNum w:abstractNumId="0"/><w:abstractNum w:abstractNumId="0"/><w:num w:numId="1"><w:abstractNumId w:val="9"/></w:num></w:numbering>"#
        ),
    );
    d.set_doc_rels(
        r#"<Relationship Id="rN" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/>"#,
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass);
    assert_finding(&report, "NUM-DANGLING", Some("word/numbering.xml"));
}

#[test]
fn num_matched_does_not_fire() {
    let mut d = Doc::minimal();
    d.add_with_override(
        "word/numbering.xml",
        CT_NUMBERING,
        format!(
            r#"<?xml version="1.0"?><w:numbering xmlns:w="{W_NS}"><w:abstractNum w:abstractNumId="0"/><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#
        ),
    );
    d.set_doc_rels(
        r#"<Relationship Id="rN" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/>"#,
    );
    d.set_document(r#"<w:p><w:pPr><w:numPr><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>x</w:t></w:r></w:p>"#);
    let report = validate(&d);
    assert!(!has(&report, "NUM-DANGLING"), "{:?}", report.findings);
}

#[test]
fn num_id_zero_is_not_dangling() {
    let mut d = Doc::minimal();
    d.set_document(r#"<w:p><w:pPr><w:numPr><w:numId w:val="0"/></w:numPr></w:pPr><w:r><w:t>x</w:t></w:r></w:p>"#);
    let report = validate(&d);
    assert!(!has(&report, "NUM-DANGLING"), "numId 0 is the remove-numbering sentinel: {:?}", report.findings);
}

#[test]
fn style_num_id_is_checked() {
    let mut d = Doc::minimal();
    d.add_with_override(
        "word/numbering.xml",
        CT_NUMBERING,
        format!(
            r#"<?xml version="1.0"?><w:numbering xmlns:w="{W_NS}"><w:abstractNum w:abstractNumId="0"/><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#
        ),
    );
    d.add_with_override(
        "word/styles.xml",
        CT_STYLES,
        format!(
            r#"<?xml version="1.0"?><w:styles xmlns:w="{W_NS}"><w:style w:type="paragraph" w:styleId="List"><w:pPr><w:numPr><w:numId w:val="99"/></w:numPr></w:pPr></w:style></w:styles>"#
        ),
    );
    d.set_doc_rels(concat!(
        r#"<Relationship Id="rN" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/>"#,
        r#"<Relationship Id="rS" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#,
    ));
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass);
    assert_finding(&report, "NUM-DANGLING", Some("word/styles.xml"));
}

#[test]
fn style_num_id_zero_is_not_dangling() {
    let mut d = Doc::minimal();
    d.add_with_override(
        "word/styles.xml",
        CT_STYLES,
        format!(
            r#"<?xml version="1.0"?><w:styles xmlns:w="{W_NS}"><w:style w:type="paragraph" w:styleId="List"><w:pPr><w:numPr><w:numId w:val="0"/></w:numPr></w:pPr></w:style></w:styles>"#
        ),
    );
    d.set_doc_rels(
        r#"<Relationship Id="rS" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#,
    );
    let report = validate(&d);
    assert!(!has(&report, "NUM-DANGLING"), "{:?}", report.findings);
}

#[test]
fn commentex_link() {
    let mut d = Doc::minimal();
    d.add_with_override(
        "word/comments.xml",
        CT_COMMENTS,
        format!(
            r#"<?xml version="1.0"?><w:comments xmlns:w="{W_NS}" xmlns:w14="{W14_NS}"><w:comment w:id="1"><w:p w14:paraId="AAAAAAAA"><w:r><w:t>hi</w:t></w:r></w:p></w:comment></w:comments>"#
        ),
    );
    d.add_with_override(
        "word/commentsExtended.xml",
        CT_COMMENTS_EXT,
        format!(
            r#"<?xml version="1.0"?><w15:commentsEx xmlns:w15="{W15_NS}"><w15:commentEx w15:paraId="BBBBBBBB" w15:paraIdParent="CCCCCCCC"/></w15:commentsEx>"#
        ),
    );
    d.set_doc_rels(
        r#"<Relationship Id="rC" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/><Relationship Id="rCE" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/commentsExtended" Target="commentsExtended.xml"/>"#,
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass);
    let f = assert_finding(&report, "COMMENTEX-LINK", Some("word/commentsExtended.xml"));
    assert!(f.evidence == wordcraft_validate::Evidence::Inferred);
    assert!(report.checks_run.contains(&"COMMENTEX-LINK"), "the rule runs when commentsExtended.xml exists");
}

#[test]
fn commentex_matched_does_not_fire() {
    let mut d = Doc::minimal();
    d.add_with_override(
        "word/comments.xml",
        CT_COMMENTS,
        format!(
            r#"<?xml version="1.0"?><w:comments xmlns:w="{W_NS}" xmlns:w14="{W14_NS}"><w:comment w:id="1"><w:p w14:paraId="AAAAAAAA"><w:r><w:t>hi</w:t></w:r></w:p></w:comment></w:comments>"#
        ),
    );
    d.add_with_override(
        "word/commentsExtended.xml",
        CT_COMMENTS_EXT,
        format!(r#"<?xml version="1.0"?><w15:commentsEx xmlns:w15="{W15_NS}"><w15:commentEx w15:paraId="AAAAAAAA"/></w15:commentsEx>"#),
    );
    d.set_doc_rels(
        r#"<Relationship Id="rC" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/><Relationship Id="rCE" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/commentsExtended" Target="commentsExtended.xml"/>"#,
    );
    let report = validate(&d);
    assert!(!has(&report, "COMMENTEX-LINK"), "{:?}", report.findings);
}

#[test]
fn commentex_durable_id_without_counterpart() {
    let mut d = Doc::minimal();
    d.add_with_override(
        "word/comments.xml",
        CT_COMMENTS,
        format!(
            r#"<?xml version="1.0"?><w:comments xmlns:w="{W_NS}" xmlns:w14="{W14_NS}"><w:comment w:id="1"><w:p w14:paraId="AAAAAAAA"><w:r><w:t>hi</w:t></w:r></w:p></w:comment></w:comments>"#
        ),
    );
    d.add_with_override(
        "word/commentsExtended.xml",
        CT_COMMENTS_EXT,
        format!(r#"<?xml version="1.0"?><w15:commentsEx xmlns:w15="{W15_NS}"><w15:commentEx w15:paraId="AAAAAAAA"/></w15:commentsEx>"#),
    );
    d.add_with_override(
        "word/commentsIds.xml",
        CT_COMMENTS_IDS,
        format!(
            r#"<?xml version="1.0"?><w16cid:commentsIds xmlns:w16cid="{W16CID_NS}"><w16cid:commentId w16cid:paraId="AAAAAAAA" w16cid:durableId="1"/><w16cid:commentId w16cid:paraId="ZZZZZZZZ" w16cid:durableId="2"/></w16cid:commentsIds>"#
        ),
    );
    d.set_doc_rels(concat!(
        r#"<Relationship Id="rC" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/>"#,
        r#"<Relationship Id="rCE" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/commentsExtended" Target="commentsExtended.xml"/>"#,
        r#"<Relationship Id="rCI" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/commentsIds" Target="commentsIds.xml"/>"#,
    ));
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass);
    assert_finding(&report, "COMMENTEX-LINK", Some("word/commentsIds.xml"));
}

#[test]
fn fld_unbalanced() {
    let mut d = Doc::minimal();
    d.set_document(r#"<w:p><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#);
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "FLD-UNBALANCED", Some("word/document.xml"));
}

#[test]
fn struct_tc_no_p() {
    let mut d = Doc::minimal();
    d.set_document(r#"<w:tbl><w:tr><w:tc><w:tcPr/></w:tc></w:tr></w:tbl>"#);
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "STRUCT-TC-NO-P", Some("word/document.xml"));
}

#[test]
fn cell_with_nested_table_is_not_flagged() {
    let mut d = Doc::minimal();
    d.set_document(r#"<w:tbl><w:tr><w:tc><w:tbl><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl></w:tc></w:tr></w:tbl>"#);
    let report = validate(&d);
    assert!(!has(&report, "STRUCT-TC-NO-P"), "{:?}", report.findings);
}

#[test]
fn cell_with_block_sdt_is_not_flagged() {
    let mut d = Doc::minimal();
    d.set_document(r#"<w:tbl><w:tr><w:tc><w:sdt><w:sdtContent><w:p/></w:sdtContent></w:sdt></w:tc></w:tr></w:tbl>"#);
    let report = validate(&d);
    assert!(!has(&report, "STRUCT-TC-NO-P"), "{:?}", report.findings);
}

#[test]
fn alternate_content_cell_is_not_flagged() {
    let mut d = Doc::minimal();
    d.set_document(
        r#"<w:tbl><w:tr><w:tc><mc:AlternateContent><mc:Choice Requires="w"><w:p/></mc:Choice><mc:Fallback><w:p/></mc:Fallback></mc:AlternateContent></w:tc></w:tr></w:tbl>"#,
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass, "findings: {:?}", report.findings);
    assert!(!has(&report, "STRUCT-TC-NO-P"), "a cell whose content is in mc:Choice must not be flagged: {:?}", report.findings);
    assert!(
        report.coverage_notes.iter().any(|n| n.contains("AlternateContent")),
        "the skipped branch is a coverage note: {:?}",
        report.coverage_notes
    );
}

#[test]
fn alternate_content_branches_do_not_duplicate_ids() {
    let mut d = Doc::minimal();
    d.set_document(
        r#"<w:p><mc:AlternateContent><mc:Choice Requires="w"><w:bookmarkStart w:id="1" w:name="a"/><w:bookmarkEnd w:id="1"/></mc:Choice><mc:Fallback><w:bookmarkStart w:id="1" w:name="a"/><w:bookmarkEnd w:id="1"/></mc:Fallback></mc:AlternateContent></w:p>"#,
    );
    let report = validate(&d);
    assert!(!has(&report, "ID-BOOKMARK-DUP"), "ids inside mc:AlternateContent branches are not compared: {:?}", report.findings);
    assert!(report.coverage_notes.iter().any(|n| n.contains("AlternateContent")), "{:?}", report.coverage_notes);
}

#[test]
fn struct_deltext_is_a_warning() {
    let mut d = Doc::minimal();
    d.set_document(r#"<w:p><w:del w:id="1"><w:r><w:t>x</w:t></w:r></w:del></w:p>"#);
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass);
    let f = assert_finding(&report, "STRUCT-DELTEXT", Some("word/document.xml"));
    assert_eq!(f.severity, Severity::Warning);
}

#[test]
fn struct_deltext_outside_del_is_a_warning() {
    let mut d = Doc::minimal();
    d.set_document(r#"<w:p><w:r><w:delText>x</w:delText></w:r></w:p>"#);
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass);
    let f = assert_finding(&report, "STRUCT-DELTEXT", Some("word/document.xml"));
    assert_eq!(f.severity, Severity::Warning);
}

#[test]
fn struct_move_unpaired_is_a_warning() {
    let mut d = Doc::minimal();
    d.set_document(r#"<w:p><w:moveFromRangeStart w:id="1" w:name="A"/></w:p>"#);
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass);
    let f = assert_finding(&report, "STRUCT-MOVE-UNPAIRED", Some("word/document.xml"));
    assert_eq!(f.severity, Severity::Warning);
}

#[test]
fn move_name_mismatch_fires() {
    let mut d = Doc::minimal();
    d.set_document(
        r#"<w:p><w:moveFromRangeStart w:id="1" w:name="A"/><w:moveFromRangeEnd w:id="1"/><w:moveToRangeStart w:id="2" w:name="B"/><w:moveToRangeEnd w:id="2"/></w:p>"#,
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass);
    assert_finding(&report, "STRUCT-MOVE-UNPAIRED", Some("word/document.xml"));
}

#[test]
fn move_names_matched_do_not_fire() {
    let mut d = Doc::minimal();
    d.set_document(
        r#"<w:p><w:moveFromRangeStart w:id="1" w:name="A"/><w:moveFromRangeEnd w:id="1"/><w:moveToRangeStart w:id="2" w:name="A"/><w:moveToRangeEnd w:id="2"/></w:p>"#,
    );
    let report = validate(&d);
    assert!(!has(&report, "STRUCT-MOVE-UNPAIRED"), "{:?}", report.findings);
}

#[test]
fn duplicate_move_start_id_fires() {
    let mut d = Doc::minimal();
    d.set_document(
        r#"<w:p><w:moveFromRangeStart w:id="1" w:name="A"/><w:moveFromRangeStart w:id="1" w:name="A"/><w:moveFromRangeEnd w:id="1"/><w:moveToRangeStart w:id="1" w:name="A"/><w:moveToRangeEnd w:id="1"/></w:p>"#,
    );
    let report = validate(&d);
    assert_finding(&report, "STRUCT-MOVE-UNPAIRED", Some("word/document.xml"));
}

#[test]
fn custom_xml_schema_unattached() {
    let mut d = Doc::minimal();
    d.add_with_override(
        "customXml/itemProps1.xml",
        CT_CUSTOMXML_PROPS,
        format!(
            r#"<?xml version="1.0"?><ds:datastoreItem xmlns:ds="{DS_NS}" ds:itemID="{{X}}"><ds:schemaRefs><ds:schemaRef ds:uri="http://example.com/ns"/></ds:schemaRefs></ds:datastoreItem>"#
        ),
    );
    d.add_with_override("word/settings.xml", CT_SETTINGS, format!(r#"<?xml version="1.0"?><w:settings xmlns:w="{W_NS}"/>"#));
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    let f = assert_finding(&report, "CUSTOMXML-SCHEMA", Some("customXml/itemProps1.xml"));
    assert_eq!(f.evidence, wordcraft_validate::Evidence::MsDocumented);
}

#[test]
fn custom_xml_schema_attached_does_not_fire() {
    let mut d = Doc::minimal();
    d.add_with_override(
        "customXml/itemProps1.xml",
        CT_CUSTOMXML_PROPS,
        format!(
            r#"<?xml version="1.0"?><ds:datastoreItem xmlns:ds="{DS_NS}" ds:itemID="{{X}}"><ds:schemaRefs><ds:schemaRef ds:uri="http://example.com/ns"/></ds:schemaRefs></ds:datastoreItem>"#
        ),
    );
    d.add_with_override(
        "word/settings.xml",
        CT_SETTINGS,
        format!(r#"<?xml version="1.0"?><w:settings xmlns:w="{W_NS}"><w:attachedSchema w:val="http://example.com/ns"/></w:settings>"#),
    );
    d.set_doc_rels(
        r#"<Relationship Id="rSet" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml"/>"#,
    );
    let report = validate(&d);
    assert!(!has(&report, "CUSTOMXML-SCHEMA"), "{:?}", report.findings);
}

#[test]
fn custom_xml_inside_revision_wrapper() {
    let mut d = Doc::minimal();
    d.set_document(r#"<w:p><w:ins w:id="1"><w:customXml><w:r><w:t>x</w:t></w:r></w:customXml></w:ins></w:p>"#);
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "CUSTOMXML-SCHEMA", Some("word/document.xml"));
}

#[test]
fn custom_xml_outside_wrapper_does_not_fire() {
    let mut d = Doc::minimal();
    d.set_document(r#"<w:p><w:customXml><w:r><w:t>x</w:t></w:r></w:customXml></w:p>"#);
    let report = validate(&d);
    assert!(!has(&report, "CUSTOMXML-SCHEMA"), "{:?}", report.findings);
}

#[test]
fn media_magic_is_an_error() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[("png", "image/png")], &[]));
    d.set("word/media/pic.png", b"this is not a png file at all".to_vec());
    d.set_doc_rels(
        r#"<Relationship Id="rIdI" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/pic.png"/>"#,
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    let f = assert_finding(&report, "MEDIA-MAGIC", Some("word/media/pic.png"));
    assert_eq!(f.severity, Severity::Error);
    assert_eq!(f.evidence, wordcraft_validate::Evidence::Observed);
}

#[test]
fn real_png_magic_is_fine() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[("png", "image/png")], &[]));
    d.set("word/media/pic.png", b"\x89PNG\r\n\x1a\npng body".to_vec());
    d.set_doc_rels(
        r#"<Relationship Id="rIdI" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/pic.png"/>"#,
    );
    let report = validate(&d);
    assert!(!has(&report, "MEDIA-MAGIC"));
}

#[test]
fn main_part_that_cannot_be_decompressed_is_incomplete() {
    let zip = corrupt_entry_data(&Doc::minimal().build(), "word/document.xml");
    let report = validate_bytes(&zip);
    assert_eq!(report.verdict, Verdict::Incomplete);
    assert_clean(&report);
    assert!(report.coverage_notes.iter().any(|n| n.contains("document.xml")), "{:?}", report.coverage_notes);
}

/// Finding 1 canary (a): a part the archive cannot open (an unsupported compression method) leaves
/// the run INCOMPLETE with a note naming it, and because it is registered as present-but-unchecked
/// a relationship to it must not read as dangling.
#[test]
fn unreadable_zip_entry_is_incomplete_and_not_dangling() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/word/comments.xml", CT_COMMENTS)]));
    d.set("word/comments.xml", comments(r#"<w:comment w:id="1"><w:p/></w:comment>"#));
    d.set_doc_rels(
        r#"<Relationship Id="rC" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/>"#,
    );
    let zip = claim_entry_method(&d.build(), "word/comments.xml", 12); // BZIP2: unsupported here
    let report = validate_bytes(&zip);
    assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
    assert!(report.coverage_notes.iter().any(|n| n.contains("comments.xml")), "{:?}", report.coverage_notes);
    assert!(!has(&report, "REL-DANGLING"), "an unreadable part must be present-but-unchecked, never dangling: {:?}", report.findings);
}

/// Finding 1 canary (b): a corrupt-deflate non-main story part is INCOMPLETE with the note, not a
/// silent pass and not a dangling-reference failure.
#[test]
fn corrupt_non_main_story_part_is_incomplete() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/word/comments.xml", CT_COMMENTS)]));
    d.set("word/comments.xml", comments(r#"<w:comment w:id="1"><w:p/></w:comment>"#));
    d.set_doc_rels(
        r#"<Relationship Id="rC" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/>"#,
    );
    let zip = corrupt_entry_data(&d.build(), "word/comments.xml");
    let report = validate_bytes(&zip);
    assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
    assert_clean(&report);
    assert!(report.coverage_notes.iter().any(|n| n.contains("comments.xml")), "{:?}", report.coverage_notes);
    assert!(!has(&report, "REL-DANGLING"), "a present-but-unreadable part is not dangling: {:?}", report.findings);
}

/// §1 canary (a): an unreadable entry's identity must come from the zip crate, not a second parse
/// matched by index. The archive lists a real styles part twice (so a parser that keeps both records
/// shifts every later index) followed by an unsupported-method `word/comments.xml`. The recovered
/// name must be the unsupported part's own name, the duplicated part's real content must stay
/// intact, and the relationship to the unreadable part must not read as dangling.
#[test]
fn unreadable_entry_after_a_duplicate_keeps_its_own_identity() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/word/comments.xml", CT_COMMENTS), ("/word/styles.xml", CT_STYLES)]));
    // The duplicated part is a real styles part whose content produces a finding when it is
    // retained: its `w:numPr` references a `numId` with no numbering definition (NUM-DANGLING).
    let styles = format!(
        r#"<?xml version="1.0"?><w:styles xmlns:w="{W_NS}"><w:style w:type="paragraph" w:styleId="List"><w:pPr><w:numPr><w:numId w:val="99"/></w:numPr></w:pPr></w:style></w:styles>"#
    );
    d.set("word/styles.xml", styles.clone());
    d.set("word/style0.xml", styles); // same 15-byte name length as word/styles.xml
    d.set("word/comments.xml", comments(r#"<w:comment w:id="1"><w:p/></w:comment>"#));
    d.set_doc_rels(concat!(
        r#"<Relationship Id="rC" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/>"#,
        r#"<Relationship Id="rS" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#,
    ));
    // Rename the second of the pair so it duplicates the first (same byte length), then claim an
    // unsupported compression method for the comments part so the reader cannot open it.
    let bytes = replace_all(&d.build(), b"word/style0.xml", b"word/styles.xml");
    let zip = claim_entry_method(&bytes, "word/comments.xml", 12); // BZIP2: unsupported here
    let report = validate_bytes(&zip);
    // The PKG-DUP-ENTRY Error makes this run FAIL, not PASS; the point is where the unreadable
    // entry's identity lands, not the verdict.
    assert_eq!(report.verdict, Verdict::Fail, "findings: {:?}", report.findings);
    assert!(
        report.coverage_notes.iter().any(|n| n.contains("word/comments.xml")),
        "the coverage note must name the part that actually failed: {:?}",
        report.coverage_notes
    );
    assert!(
        !report.coverage_notes.iter().any(|n| n.contains("word/styles.xml")),
        "no note may blame the duplicated part for the unreadable entry: {:?}",
        report.coverage_notes
    );
    assert!(
        !report.findings.iter().any(|f| f.rule == "REL-DANGLING" && f.part.as_deref() == Some("word/_rels/document.xml.rels")),
        "the unreadable part is present-but-unchecked, never dangling: {:?}",
        report.findings
    );
    // The duplicated part's retained CONTENT survived the index hazard: the real styles part's
    // dangling `numId` still produces its finding. A lost or blanked duplicated entry could not.
    assert_finding(&report, "NUM-DANGLING", Some("word/styles.xml"));
}

/// §2 canary: an unreadable part-level `.rels` must not make the part's `r:id` values look
/// unresolved (REL-REF-MISSING). Absence is not established from unavailable content.
#[test]
fn unreadable_part_rels_does_not_fire_ref_missing() {
    let mut d = Doc::minimal();
    d.set_document(r#"<w:p><w:hyperlink r:id="rL1"/></w:p>"#);
    d.set(
        "word/_rels/document.xml.rels",
        rels_xml(
            r#"<Relationship Id="rL1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com/" TargetMode="External"/>"#,
        ),
    );
    let zip = claim_entry_method(&d.build(), "word/_rels/document.xml.rels", 12);
    let report = validate_bytes(&zip);
    assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
    assert!(!has(&report, "REL-REF-MISSING"), "an unreadable .rels must not make r:ids look missing: {:?}", report.findings);
}

/// §2 canary: an unreadable `[Content_Types].xml` must not make every part look undeclared
/// (CT-UNDECLARED) or the main part mistyped (CT-MAIN-TYPE).
#[test]
fn unreadable_content_types_does_not_fire_undeclared() {
    let zip = claim_entry_method(&Doc::minimal().build(), "[Content_Types].xml", 12);
    let report = validate_bytes(&zip);
    assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
    assert!(!has(&report, "CT-UNDECLARED"), "an unreadable manifest must not make every part undeclared: {:?}", report.findings);
    assert!(!has(&report, "CT-MAIN-TYPE"), "an unreadable manifest must not mistype the main part: {:?}", report.findings);
}

/// §1 canary (b): an unreadable `[Content_Types].xml` in a package that ALSO carries a valid header
/// relationship and a valid image relationship (with an image element referencing the latter) must
/// not make either look mistyped. `check_typed_targets` and `check_typed_image_refs` read the
/// target's declared type through the manifest; when the manifest's content is unavailable that
/// type is unknown, not absent, so no REL-TYPED fires and the verdict is INCOMPLETE.
#[test]
fn unreadable_content_types_does_not_fire_typed_relationships() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[("png", "image/png")], &[("/word/header1.xml", CT_HEADER)]));
    add_header(&mut d);
    d.set("word/media/pic.png", b"\x89PNG\r\n\x1a\npng bytes".to_vec());
    d.set_doc_rels(concat!(
        r#"<Relationship Id="rIdH" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/>"#,
        r#"<Relationship Id="rIdI" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/pic.png"/>"#,
    ));
    // An image element references the image relationship, so both the relationship-side
    // (`check_typed_targets`) and the element-side (`check_typed_image_refs`) typed checks run.
    d.set_document(
        r#"<w:p><w:r><w:drawing><wp:inline><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:blipFill><a:blip r:embed="rIdI"/></pic:blipFill></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#,
    );
    // No w:headerReference element: the header is only a relationship, so this exercises
    // REL-TYPED's relationship-side check without involving REF-HEADER.
    let zip = claim_entry_method(&d.build(), "[Content_Types].xml", 12);
    let report = validate_bytes(&zip);
    assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
    assert!(
        !has(&report, "REL-TYPED"),
        "an unreadable manifest must not make a valid header/image relationship look mistyped: {:?}",
        report.findings
    );
}

/// §1 canary (c): an unreadable owner `.rels` (`word/_rels/document.xml.rels`) in a package whose
/// customXml part carries a schema reference the settings part would satisfy must not report
/// CUSTOMXML-SCHEMA. The settings part is discovered through that `.rels`; when it cannot be read,
/// the attached-schema set is unknown, not empty, so the schema conclusion is skipped and the run is
/// INCOMPLETE.
#[test]
fn unreadable_owner_rels_does_not_fire_custom_xml_schema() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/word/settings.xml", CT_SETTINGS), ("/customXml/itemProps1.xml", CT_CUSTOMXML_PROPS)]));
    d.set(
        "word/settings.xml",
        format!(r#"<?xml version="1.0"?><w:settings xmlns:w="{W_NS}"><w:attachedSchema w:val="http://example.com/ns"/></w:settings>"#),
    );
    d.set(
        "customXml/itemProps1.xml",
        format!(
            r#"<?xml version="1.0"?><ds:datastoreItem xmlns:ds="{DS_NS}" ds:itemID="{{X}}"><ds:schemaRefs><ds:schemaRef ds:uri="http://example.com/ns"/></ds:schemaRefs></ds:datastoreItem>"#
        ),
    );
    // The settings relationship lives in the owner's `.rels`; when that part is unreadable the
    // discovery source is gone, so the schema reference must not be reported as unattached.
    d.set_doc_rels(
        r#"<Relationship Id="rSet" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml"/>"#,
    );
    let zip = claim_entry_method(&d.build(), "word/_rels/document.xml.rels", 12);
    let report = validate_bytes(&zip);
    assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
    assert!(!has(&report, "CUSTOMXML-SCHEMA"), "an unreadable owner .rels must not make an attached schema look unattached: {:?}", report.findings);
}

/// §1 by-construction guard: with no owner scope (no main part), the discovery source for settings
/// is the manifest; when both `_rels/.rels` and `[Content_Types].xml` are unreadable, no main part
/// exists and the no-owner scope applies, and a customXml part carrying a schema reference must not
/// fire CUSTOMXML-SCHEMA — the predicate pushes `None` (skip) instead of an empty schema set. Note
/// this test cannot distinguish the guard by mutation: the second loop enumerates customXml parts by
/// content type (lib.rs), so an unreadable manifest enumerates none and the false finding cannot
/// arise either way. It pins the invariant the guard keeps by construction.
#[test]
fn no_owner_manifest_unavailable_does_not_fire_custom_xml_schema() {
    let mut d = Doc::minimal();
    d.add_with_override(
        "customXml/itemProps1.xml",
        CT_CUSTOMXML_PROPS,
        format!(
            r#"<?xml version="1.0"?><ds:datastoreItem xmlns:ds="{DS_NS}" ds:itemID="{{X}}"><ds:schemaRefs><ds:schemaRef ds:uri="http://example.com/ns"/></ds:schemaRefs></ds:datastoreItem>"#
        ),
    );
    d.add_with_override(
        "word/settings.xml",
        CT_SETTINGS,
        format!(r#"<?xml version="1.0"?><w:settings xmlns:w="{W_NS}"><w:attachedSchema w:val="http://example.com/ns"/></w:settings>"#),
    );
    // `_rels/.rels` unreadable removes the main part, so only the no-owner scope remains; the
    // manifest unreadable is the discovery source the guard must not treat as "no settings".
    let zip = claim_entry_method(&d.build(), "_rels/.rels", 12);
    let zip = claim_entry_method(&zip, "[Content_Types].xml", 12);
    let report = validate_bytes(&zip);
    assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
    assert!(!has(&report, "CUSTOMXML-SCHEMA"), "an unreadable manifest must not make an attached schema look unattached: {:?}", report.findings);
}

/// Opus N5: the "custom XML schema references were not checked" note is recorded only for an owner
/// that actually has customXml parts. With an unreadable owner `.rels` and no customXml part there
/// is nothing to lose, so no note; with a customXml part the note is kept. No finding changes.
#[test]
fn unreadable_owner_rels_note_only_for_owners_with_custom_xml() {
    const PHRASE: &str = "custom XML schema references were not checked";
    let without = Doc::minimal();
    let zip = claim_entry_method(&without.build(), "word/_rels/document.xml.rels", 12);
    let report = validate_bytes(&zip);
    assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
    assert!(
        !report.coverage_notes.iter().any(|n| n.contains(PHRASE)),
        "an owner with no customXml part has nothing to report: {:?}",
        report.coverage_notes
    );

    let mut with = Doc::minimal();
    with.add_with_override(
        "customXml/itemProps1.xml",
        CT_CUSTOMXML_PROPS,
        format!(
            r#"<?xml version="1.0"?><ds:datastoreItem xmlns:ds="{DS_NS}" ds:itemID="{{X}}"><ds:schemaRefs><ds:schemaRef ds:uri="http://example.com/ns"/></ds:schemaRefs></ds:datastoreItem>"#
        ),
    );
    let zip = claim_entry_method(&with.build(), "word/_rels/document.xml.rels", 12);
    let report = validate_bytes(&zip);
    assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
    assert!(
        report.coverage_notes.iter().any(|n| n.contains(PHRASE)),
        "an owner with a customXml part keeps the explanatory note: {:?}",
        report.coverage_notes
    );
}

/// §2 canary: an unreadable comments part must not make the document's comment references look
/// dangling (ID-COMMENT-REF), on top of the REL-DANGLING the round-8 canary already checked.
#[test]
fn unreadable_comments_part_does_not_fire_comment_ref() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/word/comments.xml", CT_COMMENTS)]));
    d.set("word/comments.xml", comments(r#"<w:comment w:id="1"><w:p/></w:comment>"#));
    d.set_doc_rels(
        r#"<Relationship Id="rC" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/>"#,
    );
    d.set_document(r#"<w:p><w:r><w:commentReference w:id="1"/></w:r></w:p>"#);
    let zip = claim_entry_method(&d.build(), "word/comments.xml", 12);
    let report = validate_bytes(&zip);
    assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
    assert!(!has(&report, "REL-DANGLING"), "{:?}", report.findings);
    assert!(!has(&report, "ID-COMMENT-REF"), "an unreadable comments part must not make references dangling: {:?}", report.findings);
}

/// §1 residual canary: an unreadable owner `.rels` (`word/_rels/document.xml.rels`) plus a comments
/// part that is NOT discoverable by content type (typed only by the `xml` default, `application/xml`
/// — never the comments content type) must not make the document's comment references look dangling
/// (`ID-COMMENT-REF`). The supporting part's existence is unestablished, not established-absent, so
/// no dangling finding fires and the run is INCOMPLETE from the unreadable `.rels`. Removing the
/// guard restores the false FAIL (`ID-COMMENT-REF`, Error).
#[test]
fn unreadable_owner_rels_with_untyped_comments_does_not_fire_comment_ref() {
    let mut d = Doc::minimal();
    // The comments part exists and the owner's `.rels` names it, but no Override types it: it falls
    // back to the `xml` Default (`application/xml`), so it is not discoverable by content type.
    d.set("word/comments.xml", comments(r#"<w:comment w:id="1"><w:p/></w:comment>"#));
    d.set_doc_rels(
        r#"<Relationship Id="rC" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/>"#,
    );
    d.set_document(r#"<w:p><w:r><w:commentReference w:id="1"/></w:r></w:p>"#);
    let zip = claim_entry_method(&d.build(), "word/_rels/document.xml.rels", 12);
    let report = validate_bytes(&zip);
    assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
    assert!(!has(&report, "ID-COMMENT-REF"), "an unreadable owner .rels must not make references dangling: {:?}", report.findings);
}

/// §2 canary: an unreadable `_rels/.rels` must not make main resolution report PKG-NO-MAIN. The
/// officeDocument relationship is unknown, not absent.
#[test]
fn unreadable_package_rels_does_not_fire_no_main() {
    let zip = claim_entry_method(&Doc::minimal().build(), "_rels/.rels", 12);
    let report = validate_bytes(&zip);
    assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
    assert!(!has(&report, "PKG-NO-MAIN"), "an unreadable _rels/.rels must not report no main: {:?}", report.findings);
}

/// §4 canary: `w14:paraId` / `w15:paraId` are ST_LongHexNumber, so the same number in different
/// case across comments.xml and commentsExtended.xml must still join (no COMMENTEX-LINK).
#[test]
fn commentex_hex_identifiers_join_despite_case() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/word/comments.xml", CT_COMMENTS), ("/word/commentsExtended.xml", CT_COMMENTS_EXT)]));
    d.set(
        "word/comments.xml",
        format!(r#"<?xml version="1.0"?><w:comments xmlns:w="{W_NS}" xmlns:w14="{W14_NS}"><w:comment w:id="1"><w:p w14:paraId="AAAA0001"/></w:comment></w:comments>"#),
    );
    d.set(
        "word/commentsExtended.xml",
        format!(r#"<?xml version="1.0"?><w15:commentsEx xmlns:w15="{W15_NS}"><w15:commentEx w15:paraId="aaaa0001"/></w15:commentsEx>"#),
    );
    d.set_doc_rels(concat!(
        r#"<Relationship Id="rC" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/>"#,
        r#"<Relationship Id="rCE" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/commentsExtended" Target="commentsExtended.xml"/>"#,
    ));
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass, "findings: {:?}", report.findings);
    assert!(!has(&report, "COMMENTEX-LINK"), "hex identifiers that differ only in case must join: {:?}", report.findings);
}

#[test]
fn validate_path_on_missing_file_is_an_io_error() {
    assert!(validate_path(Path::new("/definitely/not/a/real/file.docx")).is_err());
}

fn docx_files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return found };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(docx_files(&path));
        } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("docx")) {
            found.push(path);
        }
    }
    found
}

#[test]
fn tc_inside_alternate_content_is_not_flagged() {
    let mut d = Doc::minimal();
    d.set_document(concat!(
        r#"<mc:AlternateContent><mc:Choice Requires="w"><w:tbl><w:tr><w:tc><w:tcPr/></w:tc></w:tr></w:tbl></mc:Choice></mc:AlternateContent>"#,
        r#"<w:tbl><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>"#,
    ));
    let report = validate(&d);
    assert!(!has(&report, "STRUCT-TC-NO-P"), "a cell inside a skipped branch must not be flagged: {:?}", report.findings);
}

#[test]
fn del_close_inside_alternate_content_does_not_suppress_later_finding() {
    let mut d = Doc::minimal();
    d.set_document(concat!(
        r#"<mc:AlternateContent><mc:Choice Requires="w"><w:del w:id="1"><w:r><w:t>x</w:t></w:r></w:del></mc:Choice></mc:AlternateContent>"#,
        r#"<w:p><w:del w:id="2"><w:r><w:t>y</w:t></w:r></w:del></w:p>"#,
    ));
    let report = validate(&d);
    assert_finding(&report, "STRUCT-DELTEXT", Some("word/document.xml"));
    assert_eq!(report.verdict, Verdict::Pass, "a surviving late finding must be a warning-only run: {:?}", report.findings);
}

#[test]
fn utf16_styles_numbering_reference_is_checked() {
    let mut d = Doc::minimal();
    let styles = format!(
        r#"<?xml version="1.0" encoding="UTF-16"?><w:styles xmlns:w="{W_NS}"><w:style w:type="paragraph" w:styleId="List"><w:pPr><w:numPr><w:numId w:val="99"/></w:numPr></w:pPr></w:style></w:styles>"#
    );
    d.add_with_override("word/styles.xml", CT_STYLES, utf16_le_with_bom(&styles));
    d.set_doc_rels(
        r#"<Relationship Id="rS" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#,
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass, "findings: {:?}", report.findings);
    assert_finding(&report, "NUM-DANGLING", Some("word/styles.xml"));
}

#[test]
fn invalid_utf8_part_is_malformed() {
    let mut d = Doc::minimal();
    let mut bytes = format!(r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="{W_NS}"><w:body><w:p>"#).into_bytes();
    bytes.push(0xFF);
    bytes.extend_from_slice(b"</w:p></w:body></w:document>");
    d.set("word/document.xml", bytes);
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    let f = assert_finding(&report, "XML-MALFORMED", Some("word/document.xml"));
    assert_eq!(f.severity, Severity::Corrupt);
}

#[test]
fn unsupported_encoding_part_is_incomplete() {
    let mut d = Doc::minimal();
    d.set("word/junk.xml", br#"<?xml version="1.0" encoding="windows-1252"?><junk/>"#.to_vec());
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
    assert!(report.coverage_notes.iter().any(|n| n.contains("windows-1252")), "{:?}", report.coverage_notes);
    assert!(
        report.coverage_notes.iter().any(|n| n.contains("word/junk.xml")),
        "the unsupported-encoding note must name the affected part: {:?}",
        report.coverage_notes
    );
}

#[test]
fn reference_after_root_is_malformed() {
    let mut d = Doc::minimal();
    d.set_document_raw(&format!(r#"<?xml version="1.0"?><w:document xmlns:w="{W_NS}"><w:body/></w:document>&#65;"#));
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "XML-MALFORMED", Some("word/document.xml"));
}

#[test]
fn reference_before_root_is_malformed() {
    let mut d = Doc::minimal();
    d.set_document_raw(&format!(r#"<?xml version="1.0"?>&#65;<w:document xmlns:w="{W_NS}"><w:body/></w:document>"#));
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "XML-MALFORMED", Some("word/document.xml"));
}

#[test]
fn normal_character_reference_is_not_malformed() {
    let mut d = Doc::minimal();
    d.set_document(r#"<w:p><w:r><w:t>&#65;</w:t></w:r></w:p>"#);
    let report = validate(&d);
    assert!(!has(&report, "XML-MALFORMED"), "{:?}", report.findings);
    assert!(!has(&report, "XML-INVALID-CHAR"), "{:?}", report.findings);
}

#[test]
fn header_reference_to_external_relationship_fires() {
    let mut d = Doc::minimal();
    d.set_doc_rels(
        r#"<Relationship Id="rIdH" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="https://example.com/header.xml" TargetMode="External"/>"#,
    );
    d.set_document(r#"<w:p/><w:sectPr><w:headerReference w:type="default" r:id="rIdH"/></w:sectPr>"#);
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "REF-HEADER", Some("word/document.xml"));
}

const A_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const R_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

#[test]
fn blip_to_header_relationship_fires_rel_typed() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", rich_content_types());
    add_header(&mut d);
    d.set_doc_rels(
        r#"<Relationship Id="rIdH" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/>"#,
    );
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="{W_NS}" xmlns:r="{R_NS}" xmlns:a="{A_NS}"><w:body><w:p><w:r><w:drawing><a:blip r:embed="rIdH"/></w:drawing></w:r></w:p></w:body></w:document>"#
    );
    d.set("word/document.xml", doc.into_bytes());
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "REL-TYPED", Some("word/document.xml"));
}

#[test]
fn blip_to_image_relationship_does_not_fire() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[("png", "image/png")], &[]));
    d.set("word/media/pic.png", b"\x89PNG\r\n\x1a\npng body".to_vec());
    d.set_doc_rels(
        r#"<Relationship Id="rIdI" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/pic.png"/>"#,
    );
    let doc = format!(
        r#"<?xml version="1.0"?><w:document xmlns:w="{W_NS}" xmlns:r="{R_NS}" xmlns:a="{A_NS}"><w:body><w:p><w:r><w:drawing><a:blip r:embed="rIdI"/></w:drawing></w:r></w:p></w:body></w:document>"#
    );
    d.set("word/document.xml", doc.into_bytes());
    let report = validate(&d);
    assert!(!has(&report, "REL-TYPED"), "{:?}", report.findings);
}

#[test]
fn blip_from_header_to_image_does_not_fire() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[("png", "image/png")], &[("/word/header1.xml", CT_HEADER)]));
    d.set("word/media/pic.png", b"\x89PNG\r\n\x1a\npng body".to_vec());
    d.set_doc_rels(
        r#"<Relationship Id="rIdH" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/>"#,
    );
    d.set(
        "word/_rels/header1.xml.rels",
        rels_xml(
            r#"<Relationship Id="rIdI" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/pic.png"/>"#,
        ),
    );
    let hdr = format!(
        r#"<?xml version="1.0"?><w:hdr xmlns:w="{W_NS}" xmlns:r="{R_NS}" xmlns:a="{A_NS}"><w:p><w:r><w:drawing><a:blip r:embed="rIdI"/></w:drawing></w:r></w:p></w:hdr>"#
    );
    d.set("word/header1.xml", hdr.into_bytes());
    let report = validate(&d);
    assert!(!has(&report, "REL-TYPED"), "an image reference to a proper image rel must not fire: {:?}", report.findings);
}

fn glossary_doc(gloss_styles_first: bool) -> Doc {
    let mut d = Doc::minimal();
    d.set(
        "[Content_Types].xml",
        content_types_xml(
            &[],
            &[
                ("/word/glossary.xml", CT_GLOSSARY),
                ("/word/stylesMain.xml", CT_STYLES),
                ("/word/stylesGloss.xml", CT_STYLES),
                ("/word/numberingMain.xml", CT_NUMBERING),
                ("/word/numberingGloss.xml", CT_NUMBERING),
            ],
        ),
    );
    d.set_doc_rels(concat!(
        r#"<Relationship Id="rS" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="stylesMain.xml"/>"#,
        r#"<Relationship Id="rN" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numberingMain.xml"/>"#,
        r#"<Relationship Id="rG" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/glossaryDocument" Target="glossary.xml"/>"#,
    ));
    d.set(
        "word/_rels/glossary.xml.rels",
        rels_xml(concat!(
            r#"<Relationship Id="rS" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="stylesGloss.xml"/>"#,
            r#"<Relationship Id="rN" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numberingGloss.xml"/>"#,
        )),
    );
    d.set(
        "word/glossary.xml",
        format!(r#"<?xml version="1.0"?><w:glossaryDocument xmlns:w="{W_NS}"><w:docParts/></w:glossaryDocument>"#).into_bytes(),
    );
    let main_styles = format!(
        r#"<?xml version="1.0"?><w:styles xmlns:w="{W_NS}"><w:style w:type="paragraph" w:styleId="A"><w:pPr><w:numPr><w:numId w:val="99"/></w:numPr></w:pPr></w:style></w:styles>"#
    )
    .into_bytes();
    let gloss_styles = format!(
        r#"<?xml version="1.0"?><w:styles xmlns:w="{W_NS}"><w:style w:type="paragraph" w:styleId="B"><w:pPr><w:numPr><w:numId w:val="1"/></w:numPr></w:pPr></w:style></w:styles>"#
    )
    .into_bytes();
    let numbering =
        format!(r#"<?xml version="1.0"?><w:numbering xmlns:w="{W_NS}"><w:abstractNum w:abstractNumId="0"/><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#)
            .into_bytes();
    if gloss_styles_first {
        d.set("word/stylesGloss.xml", gloss_styles);
        d.set("word/stylesMain.xml", main_styles);
    } else {
        d.set("word/stylesMain.xml", main_styles);
        d.set("word/stylesGloss.xml", gloss_styles);
    }
    d.set("word/numberingMain.xml", numbering.clone());
    d.set("word/numberingGloss.xml", numbering);
    d
}

#[test]
fn main_styles_are_used_regardless_of_glossary_styles_order() {
    for gloss_first in [true, false] {
        let report = validate(&glossary_doc(gloss_first));
        assert_eq!(report.verdict, Verdict::Pass, "order {gloss_first}: {:?}", report.findings);
        assert_finding(&report, "NUM-DANGLING", Some("word/stylesMain.xml"));
        assert!(
            !report.findings.iter().any(|f| f.rule == "NUM-DANGLING" && f.part.as_deref() == Some("word/stylesGloss.xml")),
            "order {gloss_first}: {:?}",
            report.findings
        );
    }
}

#[test]
fn glossary_dangling_comment_reference_fires_at_glossary_part() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/word/glossary.xml", CT_GLOSSARY)]));
    d.set_doc_rels(r#"<Relationship Id="rG" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/glossaryDocument" Target="glossary.xml"/>"#);
    d.set(
        "word/glossary.xml",
        format!(
            r#"<?xml version="1.0"?><w:glossaryDocument xmlns:w="{W_NS}"><w:docParts><w:docPart><w:p><w:r><w:commentReference w:id="7"/></w:r></w:p></w:docPart></w:docParts></w:glossaryDocument>"#
        )
        .into_bytes(),
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "ID-COMMENT-REF", Some("word/glossary.xml"));
}

#[test]
fn renamed_item_props_part_gets_schema_finding() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/customXml/props.xml", CT_CUSTOMXML_PROPS)]));
    d.set(
        "customXml/props.xml",
        format!(
            r#"<?xml version="1.0"?><ds:datastoreItem xmlns:ds="{DS_NS}" ds:itemID="{{X}}"><ds:schemaRefs><ds:schemaRef ds:uri="http://example.com/ns"/></ds:schemaRefs></ds:datastoreItem>"#
        )
        .into_bytes(),
    );
    d.add_with_override("word/settings.xml", CT_SETTINGS, format!(r#"<?xml version="1.0"?><w:settings xmlns:w="{W_NS}"/>"#));
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail);
    assert_finding(&report, "CUSTOMXML-SCHEMA", Some("customXml/props.xml"));
}

#[test]
fn corpus_hook() {
    let Ok(dir) = std::env::var("WORDCRAFT_VALIDATE_CORPUS") else {
        return;
    };
    let files = docx_files(Path::new(&dir));
    assert!(
        !files.is_empty(),
        "WORDCRAFT_VALIDATE_CORPUS is set to {dir} but it contains no .docx files; an empty configured corpus is a broken gate"
    );
    let mut checked = 0;
    for path in files {
        let report = validate_path(&path).expect("the corpus file reads");
        assert_clean(&report);
        assert_ne!(report.verdict, Verdict::Incomplete, "{} was not fully checked: {:?}", path.display(), report.coverage_notes);
        checked += 1;
    }
    eprintln!("wordcraft-validate corpus: {checked} documents, no Corrupt/Error findings");
}

#[test]
fn owner_scoped_numbering_references_fire_at_their_own_part() {
    for reverse in [false, true] {
        let mut d = Doc::minimal();
        d.set(
            "[Content_Types].xml",
            content_types_xml(
                &[],
                &[("/word/glossary.xml", CT_GLOSSARY), ("/word/numberingMain.xml", CT_NUMBERING), ("/word/numberingGloss.xml", CT_NUMBERING)],
            ),
        );
        d.set_doc_rels(concat!(
            r#"<Relationship Id="rN" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numberingMain.xml"/>"#,
            r#"<Relationship Id="rG" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/glossaryDocument" Target="glossary.xml"/>"#,
        ));
        d.set(
            "word/_rels/glossary.xml.rels",
            rels_xml(r#"<Relationship Id="rN" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numberingGloss.xml"/>"#),
        );
        d.set(
            "word/glossary.xml",
            format!(
                r#"<?xml version="1.0"?><w:glossaryDocument xmlns:w="{W_NS}"><w:docParts><w:docPart><w:p><w:pPr><w:numPr><w:numId w:val="88"/></w:numPr></w:pPr></w:p></w:docPart></w:docParts></w:glossaryDocument>"#
            ),
        );
        d.set_document(r#"<w:p><w:pPr><w:numPr><w:numId w:val="77"/></w:numPr></w:pPr><w:r><w:t>x</w:t></w:r></w:p>"#);
        d.set(
            "word/numberingMain.xml",
            format!(
                r#"<?xml version="1.0"?><w:numbering xmlns:w="{W_NS}"><w:abstractNum w:abstractNumId="0"/><w:num w:numId="88"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#
            ),
        );
        d.set(
            "word/numberingGloss.xml",
            format!(
                r#"<?xml version="1.0"?><w:numbering xmlns:w="{W_NS}"><w:abstractNum w:abstractNumId="0"/><w:num w:numId="77"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#
            ),
        );
        if reverse {
            d.entries.reverse();
        }
        let report = validate(&d);
        assert_eq!(report.verdict, Verdict::Pass, "reverse {reverse}: {:?}", report.findings);
        assert_finding(&report, "NUM-DANGLING", Some("word/document.xml"));
        assert_finding(&report, "NUM-DANGLING", Some("word/glossary.xml"));
    }
}

#[test]
fn owner_scoped_style_references_fire_at_their_own_part() {
    for reverse in [false, true] {
        let mut d = Doc::minimal();
        d.set(
            "[Content_Types].xml",
            content_types_xml(&[], &[("/word/glossary.xml", CT_GLOSSARY), ("/word/stylesMain.xml", CT_STYLES), ("/word/stylesGloss.xml", CT_STYLES)]),
        );
        d.set_doc_rels(concat!(
            r#"<Relationship Id="rS" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="stylesMain.xml"/>"#,
            r#"<Relationship Id="rG" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/glossaryDocument" Target="glossary.xml"/>"#,
        ));
        d.set(
            "word/_rels/glossary.xml.rels",
            rels_xml(r#"<Relationship Id="rS" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="stylesGloss.xml"/>"#),
        );
        d.set(
            "word/glossary.xml",
            format!(
                r#"<?xml version="1.0"?><w:glossaryDocument xmlns:w="{W_NS}"><w:docParts><w:docPart><w:p><w:pPr><w:pStyle w:val="MainOnly"/></w:pPr></w:p></w:docPart></w:docParts></w:glossaryDocument>"#
            ),
        );
        d.set_document(r#"<w:p><w:pPr><w:pStyle w:val="GlossOnly"/></w:pPr><w:r><w:t>x</w:t></w:r></w:p>"#);
        d.set(
            "word/stylesMain.xml",
            format!(r#"<?xml version="1.0"?><w:styles xmlns:w="{W_NS}"><w:style w:type="paragraph" w:styleId="MainOnly"><w:name w:val="MainOnly"/></w:style></w:styles>"#),
        );
        d.set(
            "word/stylesGloss.xml",
            format!(r#"<?xml version="1.0"?><w:styles xmlns:w="{W_NS}"><w:style w:type="paragraph" w:styleId="GlossOnly"><w:name w:val="GlossOnly"/></w:style></w:styles>"#),
        );
        if reverse {
            d.entries.reverse();
        }
        let report = validate(&d);
        assert_eq!(report.verdict, Verdict::Pass, "reverse {reverse}: {:?}", report.findings);
        assert_finding(&report, "REF-STYLE", Some("word/document.xml"));
        assert_finding(&report, "REF-STYLE", Some("word/glossary.xml"));
    }
}

#[test]
fn missing_owner_relationship_does_not_widen_the_scope() {
    // Neither owner reaches a styles part through a relationship, yet a styles part exists by
    // content type and both owners carry style references. A first-by-content-type fallback would
    // let ZIP order decide which references are accepted (J2): the references must stay unchecked
    // in both orders, so no REF-STYLE may fire at either owner. The intentional no-finding case is
    // asserted explicitly, with the verdict.
    for reverse in [false, true] {
        let mut d = Doc::minimal();
        d.set(
            "[Content_Types].xml",
            content_types_xml(&[], &[("/word/glossary.xml", CT_GLOSSARY), ("/word/stylesMain.xml", CT_STYLES), ("/word/stylesGloss.xml", CT_STYLES)]),
        );
        d.set_doc_rels(r#"<Relationship Id="rG" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/glossaryDocument" Target="glossary.xml"/>"#);
        d.set("word/_rels/glossary.xml.rels", rels_xml(""));
        d.set(
            "word/glossary.xml",
            format!(
                r#"<?xml version="1.0"?><w:glossaryDocument xmlns:w="{W_NS}"><w:docParts><w:docPart><w:p><w:pPr><w:pStyle w:val="GlossOnly"/></w:pPr></w:p></w:docPart></w:docParts></w:glossaryDocument>"#
            ),
        );
        d.set_document(r#"<w:p><w:pPr><w:pStyle w:val="MainOnly"/></w:pPr><w:r><w:t>x</w:t></w:r></w:p>"#);
        d.set(
            "word/stylesMain.xml",
            format!(r#"<?xml version="1.0"?><w:styles xmlns:w="{W_NS}"><w:style w:type="paragraph" w:styleId="MainOnly"><w:name w:val="MainOnly"/></w:style></w:styles>"#),
        );
        d.set(
            "word/stylesGloss.xml",
            format!(r#"<?xml version="1.0"?><w:styles xmlns:w="{W_NS}"><w:style w:type="paragraph" w:styleId="GlossOnly"><w:name w:val="GlossOnly"/></w:style></w:styles>"#),
        );
        if reverse {
            d.entries.reverse();
        }
        let report = validate(&d);
        assert_eq!(report.verdict, Verdict::Pass, "reverse {reverse}: {:?}", report.findings);
        assert!(
            !report.findings.iter().any(|f| f.rule == "REF-STYLE" && f.part.as_deref() == Some("word/document.xml")),
            "reverse {reverse}: a main reference must not widen into a by-content-type styles part: {:?}",
            report.findings
        );
        assert!(
            !report.findings.iter().any(|f| f.rule == "REF-STYLE" && f.part.as_deref() == Some("word/glossary.xml")),
            "reverse {reverse}: a glossary reference must not widen into a by-content-type styles part: {:?}",
            report.findings
        );
    }
}

#[test]
fn skipped_alternate_content_style_num_id_does_not_fire() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/word/numbering.xml", CT_NUMBERING), ("/word/styles.xml", CT_STYLES)]));
    d.set(
        "word/numbering.xml",
        format!(
            r#"<?xml version="1.0"?><w:numbering xmlns:w="{W_NS}"><w:abstractNum w:abstractNumId="0"/><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#
        ),
    );
    d.set(
        "word/styles.xml",
        format!(
            r#"<?xml version="1.0"?><w:styles xmlns:w="{W_NS}" xmlns:mc="{MC_NS}"><w:style w:type="paragraph" w:styleId="List"><w:pPr><mc:AlternateContent><mc:Choice Requires="w"><w:numPr><w:numId w:val="99"/></w:numPr></mc:Choice></mc:AlternateContent><w:numPr><w:numId w:val="98"/></w:numPr></w:pPr></w:style></w:styles>"#
        ),
    );
    d.set_doc_rels(concat!(
        r#"<Relationship Id="rN" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/>"#,
        r#"<Relationship Id="rS" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#,
    ));
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass, "findings: {:?}", report.findings);
    let findings: Vec<&Finding> = report.findings.iter().filter(|f| f.rule == "NUM-DANGLING").collect();
    assert_eq!(findings.len(), 1, "the skipped branch must not fire: {:?}", report.findings);
    assert_finding(&report, "NUM-DANGLING", Some("word/styles.xml"));
    assert!(findings[0].message.contains("98"), "{}", findings[0].message);
    assert!(!findings.iter().any(|f| f.message.contains("99")), "{:?}", report.findings);
}

#[test]
fn blip_embed_to_external_image_relationship_fires() {
    let mut d = Doc::minimal();
    d.set_doc_rels(
        r#"<Relationship Id="rIdX" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="https://example.com/pic.png" TargetMode="External"/>"#,
    );
    let doc = format!(
        r#"<?xml version="1.0"?><w:document xmlns:w="{W_NS}" xmlns:r="{R_NS}" xmlns:a="{A_NS}"><w:body><w:p><w:r><w:drawing><a:blip r:embed="rIdX"/></w:drawing></w:r></w:p></w:body></w:document>"#
    );
    d.set("word/document.xml", doc.into_bytes());
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail, "findings: {:?}", report.findings);
    assert_finding(&report, "REL-TYPED", Some("word/document.xml"));
}

#[test]
fn blip_embed_from_header_to_external_relationship_fires() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/word/header1.xml", CT_HEADER)]));
    d.set(
        "word/_rels/header1.xml.rels",
        rels_xml(
            r#"<Relationship Id="rIdX" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="https://example.com/pic.png" TargetMode="External"/>"#,
        ),
    );
    let hdr = format!(
        r#"<?xml version="1.0"?><w:hdr xmlns:w="{W_NS}" xmlns:r="{R_NS}" xmlns:a="{A_NS}"><w:p><w:r><w:drawing><a:blip r:embed="rIdX"/></w:drawing></w:r></w:p></w:hdr>"#
    );
    d.set("word/header1.xml", hdr.into_bytes());
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail, "findings: {:?}", report.findings);
    assert_finding(&report, "REL-TYPED", Some("word/header1.xml"));
}

#[test]
fn blip_link_to_external_image_relationship_does_not_fire() {
    let mut d = Doc::minimal();
    d.set_doc_rels(
        r#"<Relationship Id="rIdX" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="https://example.com/pic.png" TargetMode="External"/>"#,
    );
    let doc = format!(
        r#"<?xml version="1.0"?><w:document xmlns:w="{W_NS}" xmlns:r="{R_NS}" xmlns:a="{A_NS}"><w:body><w:p><w:r><w:drawing><a:blip r:link="rIdX"/></w:drawing></w:r></w:p></w:body></w:document>"#
    );
    d.set("word/document.xml", doc.into_bytes());
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass, "a genuine external link is legitimate: {:?}", report.findings);
    assert!(!has(&report, "REL-TYPED"), "a genuine external link is legitimate: {:?}", report.findings);
}

#[test]
fn styles_part_named_without_xml_is_decoded_by_content_type() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/word/styles.dat", CT_STYLES), ("/word/numbering.xml", CT_NUMBERING)]));
    d.set_doc_rels(concat!(
        r#"<Relationship Id="rS" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.dat"/>"#,
        r#"<Relationship Id="rN" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/>"#,
    ));
    d.set(
        "word/styles.dat",
        format!(
            r#"<?xml version="1.0"?><w:styles xmlns:w="{W_NS}"><w:style w:type="paragraph" w:styleId="List"><w:pPr><w:numPr><w:numId w:val="99"/></w:numPr></w:pPr></w:style></w:styles>"#
        ),
    );
    d.set(
        "word/numbering.xml",
        format!(
            r#"<?xml version="1.0"?><w:numbering xmlns:w="{W_NS}"><w:abstractNum w:abstractNumId="0"/><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#
        ),
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass, "findings: {:?}", report.findings);
    assert_finding(&report, "NUM-DANGLING", Some("word/styles.dat"));
}

#[test]
fn custom_xml_props_part_named_without_xml_is_decoded_by_content_type() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/customXml/props.dat", CT_CUSTOMXML_PROPS)]));
    d.set(
        "customXml/props.dat",
        format!(
            r#"<?xml version="1.0"?><ds:datastoreItem xmlns:ds="{DS_NS}" ds:itemID="{{X}}"><ds:schemaRefs><ds:schemaRef ds:uri="http://example.com/ns"/></ds:schemaRefs></ds:datastoreItem>"#
        ),
    );
    d.add_with_override("word/settings.xml", CT_SETTINGS, format!(r#"<?xml version="1.0"?><w:settings xmlns:w="{W_NS}"/>"#));
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail, "findings: {:?}", report.findings);
    assert_finding(&report, "CUSTOMXML-SCHEMA", Some("customXml/props.dat"));
}

#[test]
fn long_encoding_declaration_is_incomplete_with_part_note() {
    let mut d = Doc::minimal();
    let decl = format!(r#"<?xml version="1.0" {} encoding="windows-1252"?><junk/>"#, " ".repeat(600));
    d.set("word/junk.xml", decl.into_bytes());
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
    assert!(
        report.coverage_notes.iter().any(|n| n.contains("word/junk.xml") && n.contains("windows-1252")),
        "the note must name the part and its encoding: {:?}",
        report.coverage_notes
    );
}

#[test]
fn unterminated_encoding_declaration_is_incomplete() {
    let mut d = Doc::minimal();
    let mut bytes = b"<?xml ".to_vec();
    bytes.extend(std::iter::repeat_n(b'a', (1 << 20) + 16));
    d.set("word/big.xml", bytes);
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
    assert!(
        report.coverage_notes.iter().any(|n| n.contains("word/big.xml")),
        "an unterminated declaration must name its part: {:?}",
        report.coverage_notes
    );
}

#[test]
fn unavailable_numbering_part_does_not_fire_dangling_finding() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/word/numbering.xml", CT_NUMBERING)]));
    d.set(
        "word/numbering.xml",
        format!(
            r#"<?xml version="1.0" encoding="windows-1252"?><w:numbering xmlns:w="{W_NS}"><w:abstractNum w:abstractNumId="0"/><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#
        ),
    );
    d.set_document(r#"<w:p><w:pPr><w:numPr><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>x</w:t></w:r></w:p>"#);
    d.set_doc_rels(
        r#"<Relationship Id="rN" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/>"#,
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
    assert!(
        report.coverage_notes.iter().any(|n| n.contains("word/numbering.xml")),
        "the underlying cause must name its part: {:?}",
        report.coverage_notes
    );
    assert!(!has(&report, "NUM-DANGLING"), "an unreadable numbering part must not manufacture a dangling finding: {:?}", report.findings);
}

/// §2 residual canary: an unreadable owner `.rels` (`word/_rels/document.xml.rels`) with a
/// `w:numId val="99"` in the document must not manufacture a spurious NUM-DANGLING finding. The
/// owner's numbering is unestablished (its discovery source could not be read), not established
/// absent, so no dangling finding fires and the run is INCOMPLETE from the unreadable `.rels`.
/// Removing the `None`/`Some` distinction restores the spurious Warning.
#[test]
fn unreadable_owner_rels_does_not_fire_numbering_dangling() {
    let mut d = Doc::minimal();
    d.set_document(r#"<w:p><w:pPr><w:numPr><w:numId w:val="99"/></w:numPr></w:pPr><w:r><w:t>x</w:t></w:r></w:p>"#);
    let zip = claim_entry_method(&d.build(), "word/_rels/document.xml.rels", 12);
    let report = validate_bytes(&zip);
    assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
    assert!(!has(&report, "NUM-DANGLING"), "an unreadable owner .rels must not manufacture a numbering dangling finding: {:?}", report.findings);
}

#[test]
fn comments_ids_finding_keeps_its_own_part() {
    let mut d = Doc::minimal();
    d.set(
        "[Content_Types].xml",
        content_types_xml(
            &[],
            &[
                ("/word/glossary.xml", CT_GLOSSARY),
                ("/word/commentsMain.xml", CT_COMMENTS),
                ("/word/commentsExtMain.xml", CT_COMMENTS_EXT),
                ("/word/commentsIdsMain.xml", CT_COMMENTS_IDS),
                ("/word/commentsGloss.xml", CT_COMMENTS),
                ("/word/commentsExtGloss.xml", CT_COMMENTS_EXT),
                ("/word/commentsIdsGloss.xml", CT_COMMENTS_IDS),
            ],
        ),
    );
    d.set_doc_rels(concat!(
        r#"<Relationship Id="rC" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="commentsMain.xml"/>"#,
        r#"<Relationship Id="rCE" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/commentsExtended" Target="commentsExtMain.xml"/>"#,
        r#"<Relationship Id="rCI" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/commentsIds" Target="commentsIdsMain.xml"/>"#,
        r#"<Relationship Id="rG" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/glossaryDocument" Target="glossary.xml"/>"#,
    ));
    d.set(
        "word/_rels/glossary.xml.rels",
        rels_xml(concat!(
            r#"<Relationship Id="rC" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="commentsGloss.xml"/>"#,
            r#"<Relationship Id="rCE" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/commentsExtended" Target="commentsExtGloss.xml"/>"#,
            r#"<Relationship Id="rCI" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/commentsIds" Target="commentsIdsGloss.xml"/>"#,
        )),
    );
    d.set("word/glossary.xml", format!(r#"<?xml version="1.0"?><w:glossaryDocument xmlns:w="{W_NS}"><w:docParts/></w:glossaryDocument>"#));
    d.set(
        "word/commentsMain.xml",
        format!(
            r#"<?xml version="1.0"?><w:comments xmlns:w="{W_NS}" xmlns:w14="{W14_NS}"><w:comment w:id="1"><w:p w14:paraId="AAAA0001"/></w:comment></w:comments>"#
        ),
    );
    d.set(
        "word/commentsExtMain.xml",
        format!(r#"<?xml version="1.0"?><w15:commentsEx xmlns:w15="{W15_NS}"><w15:commentEx w15:paraId="AAAA0001"/></w15:commentsEx>"#),
    );
    d.set(
        "word/commentsIdsMain.xml",
        format!(
            r#"<?xml version="1.0"?><w16cid:commentsIds xmlns:w16cid="{W16CID_NS}"><w16cid:commentId w16cid:paraId="AAAA0001" w16cid:durableId="1"/></w16cid:commentsIds>"#
        ),
    );
    d.set(
        "word/commentsGloss.xml",
        format!(
            r#"<?xml version="1.0"?><w:comments xmlns:w="{W_NS}" xmlns:w14="{W14_NS}"><w:comment w:id="2"><w:p w14:paraId="BBBB0002"/></w:comment></w:comments>"#
        ),
    );
    d.set(
        "word/commentsExtGloss.xml",
        format!(r#"<?xml version="1.0"?><w15:commentsEx xmlns:w15="{W15_NS}"><w15:commentEx w15:paraId="BBBB0002"/></w15:commentsEx>"#),
    );
    d.set(
        "word/commentsIdsGloss.xml",
        format!(
            r#"<?xml version="1.0"?><w16cid:commentsIds xmlns:w16cid="{W16CID_NS}"><w16cid:commentId w16cid:paraId="BBBB0002" w16cid:durableId="2"/><w16cid:commentId w16cid:paraId="ZZZZ9999" w16cid:durableId="3"/></w16cid:commentsIds>"#
        ),
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass, "findings: {:?}", report.findings);
    let links: Vec<&Finding> = report.findings.iter().filter(|f| f.rule == "COMMENTEX-LINK").collect();
    assert_eq!(links.len(), 1, "the unmatched durableId must fire once, at its own part: {:?}", report.findings);
    assert_eq!(links[0].part.as_deref(), Some("word/commentsIdsGloss.xml"));
}

#[test]
fn custom_xml_schemas_are_checked_per_owner() {
    // J3: a glossary-only attached schema must not satisfy a main-owned reference, nor the reverse.
    // Each customXml properties part is checked against its owning scope's settings only.
    for reverse in [false, true] {
        let mut d = Doc::minimal();
        d.set(
            "[Content_Types].xml",
            content_types_xml(
                &[],
                &[
                    ("/word/glossary.xml", CT_GLOSSARY),
                    ("/word/settingsMain.xml", CT_SETTINGS),
                    ("/word/settingsGloss.xml", CT_SETTINGS),
                    ("/customXml/itemPropsMain.xml", CT_CUSTOMXML_PROPS),
                    ("/customXml/itemPropsGloss.xml", CT_CUSTOMXML_PROPS),
                ],
            ),
        );
        d.set_doc_rels(concat!(
            r#"<Relationship Id="rSet" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settingsMain.xml"/>"#,
            r#"<Relationship Id="rG" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/glossaryDocument" Target="glossary.xml"/>"#,
        ));
        d.set(
            "word/_rels/glossary.xml.rels",
            rels_xml(concat!(
                r#"<Relationship Id="rSet" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settingsGloss.xml"/>"#,
                r#"<Relationship Id="rCx" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="../customXml/itemPropsGloss.xml"/>"#,
            )),
        );
        d.set("word/glossary.xml", format!(r#"<?xml version="1.0"?><w:glossaryDocument xmlns:w="{W_NS}"><w:docParts/></w:glossaryDocument>"#));
        d.set(
            "word/settingsMain.xml",
            format!(r#"<?xml version="1.0"?><w:settings xmlns:w="{W_NS}"><w:attachedSchema w:val="http://example.com/main"/></w:settings>"#),
        );
        d.set(
            "word/settingsGloss.xml",
            format!(r#"<?xml version="1.0"?><w:settings xmlns:w="{W_NS}"><w:attachedSchema w:val="http://example.com/gloss"/></w:settings>"#),
        );
        d.set(
            "customXml/itemPropsMain.xml",
            format!(
                r#"<?xml version="1.0"?><ds:datastoreItem xmlns:ds="{DS_NS}" ds:itemID="{{M}}"><ds:schemaRefs><ds:schemaRef ds:uri="http://example.com/gloss"/></ds:schemaRefs></ds:datastoreItem>"#
            ),
        );
        d.set(
            "customXml/itemPropsGloss.xml",
            format!(
                r#"<?xml version="1.0"?><ds:datastoreItem xmlns:ds="{DS_NS}" ds:itemID="{{G}}"><ds:schemaRefs><ds:schemaRef ds:uri="http://example.com/main"/></ds:schemaRefs></ds:datastoreItem>"#
            ),
        );
        if reverse {
            d.entries.reverse();
        }
        let report = validate(&d);
        assert_eq!(report.verdict, Verdict::Fail, "reverse {reverse}: {:?}", report.findings);
        assert_finding(&report, "CUSTOMXML-SCHEMA", Some("customXml/itemPropsMain.xml"));
        assert_finding(&report, "CUSTOMXML-SCHEMA", Some("customXml/itemPropsGloss.xml"));
    }
}

#[test]
fn non_xml_part_with_unsupported_encoding_is_incomplete() {
    // J4: a non-.xml part classified as XML by its content type has its decode error processed.
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/word/numbering.dat", CT_NUMBERING)]));
    d.set(
        "word/numbering.dat",
        format!(
            r#"<?xml version="1.0" encoding="windows-1252"?><w:numbering xmlns:w="{W_NS}"><w:abstractNum w:abstractNumId="0"/><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#
        ),
    );
    d.set_doc_rels(
        r#"<Relationship Id="rN" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.dat"/>"#,
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Incomplete, "findings: {:?}", report.findings);
    assert!(
        report.coverage_notes.iter().any(|n| n.contains("word/numbering.dat") && n.contains("windows-1252")),
        "the decode note must name the part and its encoding: {:?}",
        report.coverage_notes
    );
}

#[test]
fn non_xml_malformed_part_is_xml_malformed() {
    // J4: a non-.xml part classified as XML by its content type is run through the normal scanner.
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/word/styles.dat", CT_STYLES)]));
    d.set("word/styles.dat", format!(r#"<w:styles xmlns:w="{W_NS}"><w:style>"#));
    d.set_doc_rels(
        r#"<Relationship Id="rS" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.dat"/>"#,
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail, "findings: {:?}", report.findings);
    assert_finding(&report, "XML-MALFORMED", Some("word/styles.dat"));
}

#[test]
fn non_xml_story_part_references_are_collected() {
    // J4: a non-.xml story part is scanned, so its r:id references are collected and checked.
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/word/comments.dat", CT_COMMENTS)]));
    d.set(
        "word/comments.dat",
        format!(
            r#"<?xml version="1.0"?><w:comments xmlns:w="{W_NS}" xmlns:r="{R_NS}"><w:comment w:id="1"><w:p><w:hyperlink r:id="rId9"/></w:p></w:comment></w:comments>"#
        ),
    );
    d.set_doc_rels(
        r#"<Relationship Id="rC" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.dat"/>"#,
    );
    d.set_document(r#"<w:p><w:r><w:commentReference w:id="1"/></w:r></w:p>"#);
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail, "findings: {:?}", report.findings);
    assert_finding(&report, "REL-REF-MISSING", Some("word/comments.dat"));
}

#[test]
fn foreign_namespace_alternate_content_close_does_not_break_suppression() {
    // J5(a): inside a skipped mc:AlternateContent branch, a foreign-namespace element also named
    // AlternateContent must not decrement the suppression depth; the still-skipped sibling w:numId
    // 99 must produce no finding.
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/word/numbering.xml", CT_NUMBERING), ("/word/styles.xml", CT_STYLES)]));
    d.set(
        "word/numbering.xml",
        format!(
            r#"<?xml version="1.0"?><w:numbering xmlns:w="{W_NS}"><w:abstractNum w:abstractNumId="0"/><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#
        ),
    );
    d.set(
        "word/styles.xml",
        format!(
            r#"<?xml version="1.0"?><w:styles xmlns:w="{W_NS}" xmlns:mc="{MC_NS}" xmlns:x="urn:example:foreign"><w:style w:type="paragraph" w:styleId="List"><w:pPr><mc:AlternateContent><mc:Choice Requires="w"><x:AlternateContent><x:y/></x:AlternateContent><w:numPr><w:numId w:val="99"/></w:numPr></mc:Choice></mc:AlternateContent><w:numPr><w:numId w:val="1"/></w:numPr></w:pPr></w:style></w:styles>"#
        ),
    );
    d.set_doc_rels(concat!(
        r#"<Relationship Id="rN" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/>"#,
        r#"<Relationship Id="rS" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#,
    ));
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass, "findings: {:?}", report.findings);
    assert!(
        !report.findings.iter().any(|f| f.rule == "NUM-DANGLING" && f.part.as_deref() == Some("word/styles.xml")),
        "a foreign-namespace same-name close must not unbalance suppression: {:?}",
        report.findings
    );
}

#[test]
fn skipped_nested_num_does_not_clear_active_outer_reference() {
    // J5(b): a skipped nested w:num must not clear the active outer w:num context, so the outer
    // w:abstractNumId stays recorded and checked.
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/word/numbering.xml", CT_NUMBERING)]));
    d.set(
        "word/numbering.xml",
        format!(
            r#"<?xml version="1.0"?><w:numbering xmlns:w="{W_NS}" xmlns:mc="{MC_NS}"><w:abstractNum w:abstractNumId="0"/><w:num w:numId="1"><mc:AlternateContent><mc:Choice Requires="w"><w:num w:numId="9"></w:num></mc:Choice></mc:AlternateContent><w:abstractNumId w:val="0"/></w:num></w:numbering>"#
        ),
    );
    d.set_doc_rels(
        r#"<Relationship Id="rN" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/>"#,
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Pass, "findings: {:?}", report.findings);
    assert!(
        !report.findings.iter().any(|f| f.rule == "NUM-DANGLING" && f.part.as_deref() == Some("word/numbering.xml")),
        "the skipped nested w:num must not erase the active outer w:abstractNumId: {:?}",
        report.findings
    );
}

/// K1: an Override-only manifest — a rels Default and the main-document Override, but no `xml`
/// Default — gives `[Content_Types].xml` no effective content type and no relationship role. The
/// manifest must still be XML-classified, so an illegal reference inside it cannot escape the
/// scanner and let the package PASS.
#[test]
fn override_only_manifest_illegal_reference_is_reported_at_the_manifest() {
    let mut d = Doc::minimal();
    d.set(
        "[Content_Types].xml",
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/word/document.xml" ContentType="{CT_MAIN}"/>&#x0;</Types>"#
        ),
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail, "findings: {:?}", report.findings);
    assert_finding(&report, "XML-INVALID-CHAR", Some("[Content_Types].xml"));
}

/// K1: the same Override-only manifest, but syntactically malformed, must still be scanned for
/// well-formedness: `XML-MALFORMED` is reported for `[Content_Types].xml` (existing severity
/// policy unchanged: the manifest is reachable, so Error).
#[test]
fn override_only_manifest_malformed_is_xml_malformed() {
    let mut d = Doc::minimal();
    d.set(
        "[Content_Types].xml",
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/word/document.xml" ContentType="{CT_MAIN}"/>"#
        ),
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail, "findings: {:?}", report.findings);
    assert_finding(&report, "XML-MALFORMED", Some("[Content_Types].xml"));
}

/// J4 strengthening: a part whose ONLY XML classification is its relationship role — its content
/// type is neither an `+xml` type nor the rels type, and its name does not end in `.xml` — must
/// still have its decode error processed.
#[test]
fn relationship_only_classified_part_decode_error_is_processed() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[], &[("/customXml/item1.dat", "application/octet-stream")]));
    // Invalid UTF-8 with no BOM and no XML declaration: a strict decode failure, not a lossy read.
    d.set("customXml/item1.dat", vec![0x3C, 0xFF, 0xFE, 0x3E]);
    d.set_doc_rels(
        r#"<Relationship Id="rCx" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml" Target="../customXml/item1.dat"/>"#,
    );
    let report = validate(&d);
    assert_eq!(report.verdict, Verdict::Fail, "findings: {:?}", report.findings);
    assert_finding(&report, "XML-MALFORMED", Some("customXml/item1.dat"));
}
