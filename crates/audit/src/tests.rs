//! Golden tests build their own tiny `.docx` packages in code (contributor-original
//! fixtures): no binary files, no Microsoft material. Names are made up; the XML
//! shapes follow ECMA-376 Part 1 (WordprocessingML) and Part 2 (OPC).

use std::io::{Cursor, Write};

use crate::{AuditError, FindingKind, audit_bytes, audit_report};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const OFFICE_DOC: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";

/// Assemble a ZIP package from `(name, bytes)` entries.
fn package(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in entries {
        writer.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn content_types() -> Vec<u8> {
    br#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/></Types>"#.to_vec()
}

fn root_rels() -> Vec<u8> {
    root_rels_with("")
}

/// `_rels/.rels` with extra relationship elements (e.g. core/app properties).
fn root_rels_with(inner: &str) -> Vec<u8> {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="{RELS}"><Relationship Id="rId1" Type="{OFFICE_DOC}" Target="word/document.xml"/>{inner}</Relationships>"#
    )
    .into_bytes()
}

/// `word/_rels/document.xml.rels` with the given inner relationship elements.
fn doc_rels(inner: &str) -> Vec<u8> {
    format!(r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="{RELS}">{inner}</Relationships>"#).into_bytes()
}

fn document(body: &str) -> Vec<u8> {
    format!(r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="{W}"><w:body>{body}</w:body></w:document>"#).into_bytes()
}

fn plain_paragraph() -> Vec<u8> {
    document(r#"<w:p><w:r><w:t>Just text.</w:t></w:r></w:p>"#)
}

/// Minimal package with no hidden information: nothing to find.
fn clean_package() -> Vec<u8> {
    package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &root_rels()),
        ("word/document.xml", &plain_paragraph()),
        ("word/_rels/document.xml.rels", &doc_rels("")),
    ])
}

fn findings_of(bytes: &[u8]) -> Vec<crate::AuditFinding> {
    audit_bytes(bytes).unwrap()
}

fn ids_of(bytes: &[u8]) -> Vec<String> {
    findings_of(bytes).iter().map(|f| f.id.clone()).collect()
}

#[test]
fn finding_ids_are_stable() {
    // The public vocabulary: rename only by adding, never by changing.
    let ids: Vec<(&str, FindingKind)> = vec![
        ("author-pii/revision", FindingKind::RevisionAuthor),
        ("author-pii/comment", FindingKind::CommentAuthor),
        ("author-pii/people", FindingKind::PeopleAuthor),
        ("author-pii/core-properties", FindingKind::CorePropertiesAuthor),
        ("author-pii/extended-properties", FindingKind::ExtendedProperties),
        ("hidden-text", FindingKind::HiddenText),
        ("hidden-text/style", FindingKind::StyleHiddenText),
        ("active-content/vba", FindingKind::VbaProject),
        ("active-content/ole", FindingKind::OleEmbedding),
        ("package/orphan-part", FindingKind::OrphanPart),
    ];
    for (id, kind) in ids {
        assert_eq!(kind.id(), id);
    }
}

#[test]
fn revision_authors_found() {
    let body = r#"<w:p><w:ins w:id="10" w:author="Quill Ramsey" w:initials="QR" w:date="2026-03-04T05:06:07Z"><w:r><w:t>Added</w:t></w:r></w:ins><w:del w:id="11" w:author="Ona Vettel" w:date="2026-03-05T05:06:07Z"><w:r><w:delText>Removed</w:delText></w:r></w:del></w:p>"#;
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &root_rels()),
        ("word/document.xml", &document(body)),
        ("word/_rels/document.xml.rels", &doc_rels("")),
    ]);
    let findings = findings_of(&bytes);
    assert_eq!(findings.len(), 2);
    assert!(findings.iter().all(|f| f.id == "author-pii/revision" && f.severity == "warn"));
    assert!(findings.iter().any(|f| f.detail.contains("Quill Ramsey") && f.detail.contains("QR")));
    assert!(findings.iter().any(|f| f.detail.contains("Ona Vettel") && f.location.contains("w:del")));
}

#[test]
fn comment_authors_found() {
    let comments = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:comments xmlns:w="{W}"><w:comment w:id="1" w:author="Quill Ramsey" w:initials="QR" w:date="2026-03-01T10:00:00Z"><w:p><w:r><w:t>Check this.</w:t></w:r></w:p></w:comment><w:comment w:id="2" w:author="Ona Vettel" w:date="2026-03-02T10:00:00Z"><w:p><w:r><w:t>Agreed.</w:t></w:r></w:p></w:comment></w:comments>"#
    );
    let rels = doc_rels(
        r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/>"#,
    );
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &root_rels()),
        ("word/document.xml", &plain_paragraph()),
        ("word/_rels/document.xml.rels", &rels),
        ("word/comments.xml", &comments.into_bytes()),
    ]);
    let findings = findings_of(&bytes);
    assert_eq!(findings.len(), 2);
    assert!(findings.iter().all(|f| f.id == "author-pii/comment"));
    assert!(findings.iter().any(|f| f.detail.contains("2026-03-01")));
}

#[test]
fn comment_body_tracked_changes_found() {
    // Review case: scanning only `w:comment` attributes misses tracked
    // changes and hidden text inside the comment body. The body is story
    // content, so a comment whose only secrets sit in the body must not
    // report just its author.
    let comments = format!(
        concat!(
            r#"<?xml version="1.0" encoding="UTF-8"?><w:comments xmlns:w="{W}">"#,
            r#"<w:comment w:id="1" w:author="Quill Ramsey"><w:p>"#,
            r#"<w:ins w:id="2" w:author="Ona Vettel" w:date="2026-03-03T10:00:00Z"><w:r><w:t>Added inside a comment</w:t></w:r></w:ins>"#,
            r#"<w:r><w:rPr><w:rPrChange w:id="3" w:author="Ona Vettel" w:date="2026-03-03T10:00:00Z"/></w:rPr><w:t>Reformatted inside a comment</w:t></w:r>"#,
            r#"<w:r><w:rPr><w:vanish/></w:rPr><w:t>Hidden inside a comment</w:t></w:r>"#,
            r#"</w:p></w:comment></w:comments>"#
        ),
        W = W
    );
    let rels = doc_rels(
        r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/>"#,
    );
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &root_rels()),
        ("word/document.xml", &plain_paragraph()),
        ("word/_rels/document.xml.rels", &rels),
        ("word/comments.xml", &comments.into_bytes()),
    ]);
    let findings = findings_of(&bytes);
    assert_eq!(findings.len(), 4);
    assert!(findings.iter().any(|f| f.id == "author-pii/comment" && f.detail.contains("Quill Ramsey")));
    assert!(findings.iter().any(|f| f.id == "author-pii/revision" && f.location == "word/comments.xml <w:ins>" && f.detail.contains("Ona Vettel")));
    assert!(findings.iter().any(|f| f.id == "author-pii/revision" && f.location == "word/comments.xml <w:rPrChange>"));
    assert!(findings.iter().any(|f| f.id == "hidden-text" && f.location == "word/comments.xml <w:vanish>"));
}

#[test]
fn style_defined_hidden_text_found() {
    // Review case: `w:vanish` in styles.xml hides text without touching any
    // run. The "Secret Style" style hides text, the document defaults do,
    // and "Plain Style" sets `w:val="false"` so it must not be flagged.
    let styles = format!(
        concat!(
            r#"<?xml version="1.0" encoding="UTF-8"?><w:styles xmlns:w="{W}">"#,
            r#"<w:docDefaults><w:rPrDefault><w:rPr><w:vanish/></w:rPr></w:rPrDefault></w:docDefaults>"#,
            r#"<w:style w:type="character" w:styleId="SecretStyle"><w:name w:val="Secret Style"/><w:rPr><w:vanish/></w:rPr></w:style>"#,
            r#"<w:style w:type="character" w:styleId="PlainStyle"><w:name w:val="Plain Style"/><w:rPr><w:vanish w:val="false"/></w:rPr></w:style>"#,
            r#"</w:styles>"#
        ),
        W = W
    );
    let rels = doc_rels(
        r#"<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#,
    );
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &root_rels()),
        ("word/document.xml", &plain_paragraph()),
        ("word/_rels/document.xml.rels", &rels),
        ("word/styles.xml", &styles.into_bytes()),
    ]);
    let findings = findings_of(&bytes);
    assert_eq!(findings.len(), 2);
    assert!(findings.iter().all(|f| f.id == "hidden-text/style" && f.severity == "warn"));
    assert!(findings.iter().any(|f| f.detail.contains("Secret Style")));
    assert!(findings.iter().any(|f| f.detail.contains("document default")));
    // The part is parsed, not merely listed.
    let report = audit_report(&bytes).unwrap();
    assert!(report.scanned.contains(&"word/styles.xml".to_string()));
    assert!(!report.not_scanned.contains(&"word/styles.xml".to_string()));
}

#[test]
fn core_properties_found() {
    let core = br#"<?xml version="1.0" encoding="UTF-8"?><cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Draft terms</dc:title><dc:creator>Quill Ramsey</dc:creator><cp:lastModifiedBy>Ona Vettel</cp:lastModifiedBy></cp:coreProperties>"#;
    // Real packages reference docProps/core.xml from the package root rels.
    let rels = root_rels_with(
        r#"<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>"#,
    );
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &rels),
        ("word/document.xml", &plain_paragraph()),
        ("word/_rels/document.xml.rels", &doc_rels("")),
        ("docProps/core.xml", core),
    ]);
    let findings = findings_of(&bytes);
    assert_eq!(findings.len(), 2);
    assert!(findings.iter().all(|f| f.id == "author-pii/core-properties"));
    assert!(findings.iter().any(|f| f.detail == "Quill Ramsey" && f.location.contains("dc:creator")));
    assert!(findings.iter().any(|f| f.detail == "Ona Vettel" && f.location.contains("cp:lastModifiedBy")));
}

#[test]
fn extended_properties_are_info() {
    let app = br#"<?xml version="1.0" encoding="UTF-8"?><Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><Company>Acme Widgets</Company></Properties>"#;
    let rels = root_rels_with(
        r#"<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/>"#,
    );
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &rels),
        ("word/document.xml", &plain_paragraph()),
        ("word/_rels/document.xml.rels", &doc_rels("")),
        ("docProps/app.xml", app),
    ]);
    let findings = findings_of(&bytes);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].id, "author-pii/extended-properties");
    assert_eq!(findings[0].severity, "info");
    assert_eq!(findings[0].detail, "Acme Widgets");
}

#[test]
fn hidden_text_found() {
    // ECMA-376 §17.3.2.42: absent w:val means on; explicit false means not hidden.
    let body = concat!(
        r#"<w:p><w:r><w:rPr><w:vanish/></w:rPr><w:t>Secret one.</w:t></w:r></w:p>"#,
        r#"<w:p><w:r><w:rPr><w:vanish w:val="1"/></w:rPr><w:t>Secret two.</w:t></w:r></w:p>"#,
        r#"<w:p><w:r><w:rPr><w:vanish w:val="false"/></w:rPr><w:t>Visible.</w:t></w:r></w:p>"#,
    );
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &root_rels()),
        ("word/document.xml", &document(body)),
        ("word/_rels/document.xml.rels", &doc_rels("")),
    ]);
    let findings = findings_of(&bytes);
    assert_eq!(findings.len(), 2);
    assert!(findings.iter().all(|f| f.id == "hidden-text" && f.severity == "warn"));
}

#[test]
fn vba_and_ole_found() {
    // Real packages reference the VBA project and embeddings from document rels.
    let rels = doc_rels(concat!(
        r#"<Relationship Id="rId2" Type="http://schemas.microsoft.com/office/2006/relationships/vbaProject" Target="vbaProject.bin"/>"#,
        r#"<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/oleObject" Target="embeddings/oleObject1.bin"/>"#,
    ));
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &root_rels()),
        ("word/document.xml", &plain_paragraph()),
        ("word/_rels/document.xml.rels", &rels),
        ("word/vbaProject.bin", b"fake-macro-bytes"),
        ("word/embeddings/oleObject1.bin", b"fake-ole-bytes"),
    ]);
    let findings = findings_of(&bytes);
    assert_eq!(findings.len(), 2);
    assert!(findings.iter().any(|f| f.id == "active-content/vba" && f.location == "word/vbaProject.bin"));
    assert!(findings.iter().any(|f| f.id == "active-content/ole" && f.location == "word/embeddings/oleObject1.bin"));
}

#[test]
fn orphan_part_found() {
    // word/styles.xml is referenced; word/secret.xml is not.
    let rels = doc_rels(
        r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#,
    );
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &root_rels()),
        ("word/document.xml", &plain_paragraph()),
        ("word/_rels/document.xml.rels", &rels),
        ("word/styles.xml", br#"<?xml version="1.0"?><w:styles/>"#),
        ("word/secret.xml", br#"<?xml version="1.0"?><secret/>"#),
    ]);
    let findings = findings_of(&bytes);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].id, "package/orphan-part");
    assert_eq!(findings[0].severity, "info");
    assert_eq!(findings[0].location, "word/secret.xml");
}

#[test]
fn external_relationship_targets_are_not_orphans() {
    let rels = doc_rels(
        r#"<Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com/report" TargetMode="External"/>"#,
    );
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &root_rels()),
        ("word/document.xml", &plain_paragraph()),
        ("word/_rels/document.xml.rels", &rels),
    ]);
    assert!(findings_of(&bytes).is_empty());
}

#[test]
fn clean_file_has_no_findings() {
    assert!(findings_of(&clean_package()).is_empty());
}

#[test]
fn unusual_prefixes_still_detected() {
    // Matching is by namespace URI, not prefix (ECMA-376 allows any prefix).
    let body = r#"<w1:p><w1:ins w1:id="1" w1:author="Quill Ramsey"><w1:r><w1:t>x</w1:t></w1:r></w1:ins></w1:p>"#;
    let doc = format!(r#"<?xml version="1.0" encoding="UTF-8"?><w1:document xmlns:w1="{W}"><w1:body>{body}</w1:body></w1:document>"#);
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &root_rels()),
        ("word/document.xml", &doc.into_bytes()),
        ("word/_rels/document.xml.rels", &doc_rels("")),
    ]);
    let findings = findings_of(&bytes);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].id, "author-pii/revision");
}

#[test]
fn malformed_zip_is_an_error_not_a_panic() {
    let err = audit_bytes(b"this is not a zip file").unwrap_err();
    assert!(matches!(err, AuditError::Zip(_)), "{err:?}");
}

#[test]
fn truncated_xml_is_an_error_not_a_silent_pass() {
    // Fail closed: a document.xml we cannot parse must not report "clean".
    let broken = b"<?xml version=\"1.0\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body><w:p><w:ins w:author=\"Quill";
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &root_rels()),
        ("word/document.xml", broken),
        ("word/_rels/document.xml.rels", &doc_rels("")),
    ]);
    let err = audit_bytes(&bytes).unwrap_err();
    assert!(matches!(err, AuditError::Xml(_, _)), "{err:?}");
}

#[test]
fn missing_document_xml_is_an_error() {
    let bytes = package(&[("[Content_Types].xml", &content_types()), ("_rels/.rels", &root_rels())]);
    let err = audit_bytes(&bytes).unwrap_err();
    assert!(matches!(err, AuditError::Xml(_, _)), "{err:?}");
}

#[test]
fn findings_serialize_to_json() {
    let body = r#"<w:p><w:ins w:id="1" w:author="Quill Ramsey"><w:r><w:t>x</w:t></w:r></w:ins></w:p>"#;
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &root_rels()),
        ("word/document.xml", &document(body)),
        ("word/_rels/document.xml.rels", &doc_rels("")),
    ]);
    let findings = findings_of(&bytes);
    let json = serde_json::to_value(&findings).unwrap();
    let first = &json[0];
    assert_eq!(first["id"], serde_json::json!("author-pii/revision"));
    assert_eq!(first["severity"], serde_json::json!("warn"));
    assert!(first["location"].as_str().unwrap().contains("word/document.xml"));
    assert!(first["detail"].as_str().unwrap().contains("Quill Ramsey"));
    // Deterministic order: re-running gives the same sequence.
    assert_eq!(ids_of(&bytes), ids_of(&bytes));
}

#[test]
fn all_tracked_change_elements_report_authors() {
    // Every `CT_TrackChange` / `CT_MoveBookmark` element plus the tracked
    // property-change types carries `w:author` (ECMA-376 Part 1 §17.13.5).
    let elements = [
        "moveFrom",
        "moveTo",
        "moveFromRangeStart",
        "moveToRangeStart",
        "cellIns",
        "cellDel",
        "cellMerge",
        "customXmlInsRangeStart",
        "customXmlDelRangeStart",
        "customXmlMoveFromRangeStart",
        "customXmlMoveToRangeStart",
        "rPrChange",
        "pPrChange",
        "sectPrChange",
        "tblPrChange",
        "tblPrExChange",
        "trPrChange",
        "tcPrChange",
        "numberingChange",
    ];
    let mut body = String::from("<w:p>");
    for (i, el) in elements.iter().enumerate() {
        body.push_str(&format!(r#"<w:{el} w:id="{i}" w:author="Quill Ramsey"/>"#));
    }
    body.push_str("</w:p>");
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &root_rels()),
        ("word/document.xml", &document(&body)),
        ("word/_rels/document.xml.rels", &doc_rels("")),
    ]);
    let findings = findings_of(&bytes);
    assert_eq!(findings.len(), elements.len());
    assert!(findings.iter().all(|f| f.id == "author-pii/revision" && f.severity == "warn"));
    for el in elements {
        assert!(
            findings.iter().any(|f| f.location == format!("word/document.xml <w:{el}>") && f.detail.contains("Quill Ramsey")),
            "no finding for {el}"
        );
    }
}

#[test]
fn tbl_grid_change_is_not_an_author_finding() {
    // `w:tblGridChange` (`CT_TblGridChange`) defines no `w:author`: it must not
    // produce an author-pii finding even with an (invalid) author attribute.
    let body = r#"<w:p><w:tblGridChange w:id="1" w:author="Quill Ramsey"/></w:p>"#;
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &root_rels()),
        ("word/document.xml", &document(body)),
        ("word/_rels/document.xml.rels", &doc_rels("")),
    ]);
    assert!(findings_of(&bytes).is_empty());
}

#[test]
fn mismatched_end_tag_is_an_error() {
    let doc =
        format!(r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="{W}"><w:body><w:p><w:r></w:WRONG></w:r></w:p></w:body></w:document>"#);
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &root_rels()),
        ("word/document.xml", &doc.into_bytes()),
        ("word/_rels/document.xml.rels", &doc_rels("")),
    ]);
    assert!(matches!(audit_bytes(&bytes), Err(AuditError::Xml(_, _))));
}

#[test]
fn unclosed_element_at_eof_is_an_error() {
    let doc = format!(r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="{W}"><w:body><w:p><w:r><w:t>never closed"#);
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &root_rels()),
        ("word/document.xml", &doc.into_bytes()),
        ("word/_rels/document.xml.rels", &doc_rels("")),
    ]);
    assert!(matches!(audit_bytes(&bytes), Err(AuditError::Xml(_, _))));
}

#[test]
fn active_content_found_by_relationship_type_at_unusual_path() {
    // Reachable active content at a non-standard path: caught by relationship
    // type, and reachable means it is not an orphan.
    let rels = doc_rels(concat!(
        r#"<Relationship Id="rId2" Type="http://schemas.microsoft.com/office/2006/relationships/vbaProject" Target="custom/vba.bin"/>"#,
        r#"<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/package" Target="custom/pack.bin"/>"#,
    ));
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &root_rels()),
        ("word/document.xml", &plain_paragraph()),
        ("word/_rels/document.xml.rels", &rels),
        ("word/custom/vba.bin", b"fake-macro-bytes"),
        ("word/custom/pack.bin", b"fake-package-bytes"),
    ]);
    let findings = findings_of(&bytes);
    assert_eq!(findings.len(), 2);
    assert!(findings.iter().any(|f| f.id == "active-content/vba" && f.location == "word/custom/vba.bin"));
    assert!(findings.iter().any(|f| f.id == "active-content/ole" && f.location == "word/custom/pack.bin"));
    assert!(findings.iter().all(|f| f.id != "package/orphan-part"));
}

#[test]
fn orphan_subtree_rels_do_not_rescue_parts() {
    // Reachability is traversed from the package root: `word/secret.xml` is
    // orphaned, so its own `.rels` file must not rescue `word/hidden.xml`.
    // Both are orphans; the reachable `word/_rels/document.xml.rels` is not.
    let secret_rels = doc_rels(
        r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="hidden.xml"/>"#,
    );
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &root_rels()),
        ("word/document.xml", &plain_paragraph()),
        ("word/_rels/document.xml.rels", &doc_rels("")),
        ("word/secret.xml", br#"<?xml version="1.0"?><secret/>"#),
        ("word/_rels/secret.xml.rels", &secret_rels),
        ("word/hidden.xml", br#"<?xml version="1.0"?><hidden/>"#),
    ]);
    let findings = findings_of(&bytes);
    let orphans: Vec<&str> = findings.iter().filter(|f| f.id == "package/orphan-part").map(|f| f.location.as_str()).collect();
    assert_eq!(findings.len(), orphans.len());
    assert!(orphans.contains(&"word/secret.xml"));
    assert!(orphans.contains(&"word/_rels/secret.xml.rels"));
    assert!(orphans.contains(&"word/hidden.xml"));
    assert!(!orphans.contains(&"word/_rels/document.xml.rels"));
}

#[test]
fn multiple_root_elements_is_an_error() {
    // XML documents have exactly one root: a second top-level element is
    // malformed input, never a scannable part.
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="{W}"><w:body/></w:document><w:document xmlns:w="{W}"><w:body/></w:document>"#
    );
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &root_rels()),
        ("word/document.xml", &doc.into_bytes()),
        ("word/_rels/document.xml.rels", &doc_rels("")),
    ]);
    assert!(matches!(audit_bytes(&bytes), Err(AuditError::Xml(_, _))));
}

#[test]
fn empty_part_is_an_error() {
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &root_rels()),
        ("word/document.xml", b""),
        ("word/_rels/document.xml.rels", &doc_rels("")),
    ]);
    assert!(matches!(audit_bytes(&bytes), Err(AuditError::Xml(_, _))));
}

#[test]
fn trailing_text_after_root_is_an_error() {
    let doc = format!(r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="{W}"><w:body/></w:document>garbage"#);
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &root_rels()),
        ("word/document.xml", &doc.into_bytes()),
        ("word/_rels/document.xml.rels", &doc_rels("")),
    ]);
    assert!(matches!(audit_bytes(&bytes), Err(AuditError::Xml(_, _))));
}

#[test]
fn whitespace_outside_root_is_accepted() {
    // Comments, PIs and whitespace around the root are legal XML miscellany.
    let doc = format!("<!-- lead --><?xml version=\"1.0\" encoding=\"UTF-8\"?><w:document xmlns:w=\"{W}\"><w:body/></w:document>\n  <!-- trail -->");
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &root_rels()),
        ("word/document.xml", &doc.into_bytes()),
        ("word/_rels/document.xml.rels", &doc_rels("")),
    ]);
    assert!(findings_of(&bytes).is_empty());
}

#[test]
fn footnote_tracked_change_found() {
    // The review case for story-part coverage: a document whose ONLY
    // tracked change sits in a footnote must not report clean.
    let footnotes = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:footnotes xmlns:w="{W}"><w:footnote w:id="1"><w:p><w:ins w:id="2" w:author="Quill Ramsey"><w:r><w:t>Added in a note</w:t></w:r></w:ins></w:p></w:footnote></w:footnotes>"#
    );
    let rels = doc_rels(
        r#"<Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes" Target="footnotes.xml"/>"#,
    );
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &root_rels()),
        ("word/document.xml", &plain_paragraph()),
        ("word/_rels/document.xml.rels", &rels),
        ("word/footnotes.xml", &footnotes.into_bytes()),
    ]);
    let findings = findings_of(&bytes);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].id, "author-pii/revision");
    assert_eq!(findings[0].location, "word/footnotes.xml <w:ins>");
    assert!(findings[0].detail.contains("Quill Ramsey"));
}

#[test]
fn header_hidden_text_found() {
    // Headers are story parts too: a `w:vanish` run there hides text.
    let header = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:hdr xmlns:w="{W}"><w:p><w:r><w:rPr><w:vanish/></w:rPr><w:t>Draft</w:t></w:r></w:p></w:hdr>"#
    );
    let rels = doc_rels(
        r#"<Relationship Id="rId6" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/>"#,
    );
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &root_rels()),
        ("word/document.xml", &plain_paragraph()),
        ("word/_rels/document.xml.rels", &rels),
        ("word/header1.xml", &header.into_bytes()),
    ]);
    let findings = findings_of(&bytes);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].id, "hidden-text");
    assert_eq!(findings[0].location, "word/header1.xml <w:vanish>");
}

#[test]
fn people_part_authors_found() {
    // `word/people.xml` is the comment-author roster: names survive even
    // when the comments themselves are gone.
    let people = br#"<?xml version="1.0" encoding="UTF-8"?><w15:people xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml"><w15:person w15:author="Quill Ramsey" w15:presenceInfoId="42"/></w15:people>"#;
    let rels = doc_rels(r#"<Relationship Id="rId7" Type="http://schemas.microsoft.com/office/2011/relationships/people" Target="people.xml"/>"#);
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &root_rels()),
        ("word/document.xml", &plain_paragraph()),
        ("word/_rels/document.xml.rels", &rels),
        ("word/people.xml", people),
    ]);
    let findings = findings_of(&bytes);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].id, "author-pii/people");
    assert!(findings[0].detail.contains("Quill Ramsey"));
}

#[test]
fn custom_properties_identity_names_found() {
    // Only identity-named custom properties are metadata leakage; other
    // properties are the author's own content and stay unreported.
    let custom = br#"<?xml version="1.0" encoding="UTF-8"?><Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="2" name="Author"><vt:lpwstr>Quill Ramsey</vt:lpwstr></property><property fmtid="{D5CDD505-2E9C-101B-9397-08002B2CF9AE}" pid="3" name="DocumentNumber"><vt:lpwstr>42</vt:lpwstr></property></Properties>"#;
    let rels = root_rels_with(
        r#"<Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/custom-properties" Target="docProps/custom.xml"/>"#,
    );
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &rels),
        ("word/document.xml", &plain_paragraph()),
        ("word/_rels/document.xml.rels", &doc_rels("")),
        ("docProps/custom.xml", custom),
    ]);
    let findings = findings_of(&bytes);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].id, "author-pii/core-properties");
    assert!(findings[0].location.contains("docProps/custom.xml"));
    assert_eq!(findings[0].detail, "Quill Ramsey");
}

#[test]
fn report_lists_scan_coverage() {
    // A part the audit does not parse (here a theme) is listed as not
    // scanned, so "clean" always shows what it covered.
    let rels = doc_rels(
        r#"<Relationship Id="rId8" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="theme/theme1.xml"/>"#,
    );
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &root_rels()),
        ("word/document.xml", &plain_paragraph()),
        ("word/_rels/document.xml.rels", &rels),
        ("word/theme/theme1.xml", br#"<?xml version="1.0"?><a:theme/>"#),
    ]);
    let report = audit_report(&bytes).unwrap();
    assert!(report.findings.is_empty());
    assert!(report.scanned.contains(&"word/document.xml".to_string()));
    assert!(report.scanned.contains(&"_rels/.rels".to_string()));
    assert!(!report.scanned.contains(&"word/theme/theme1.xml".to_string()));
    assert!(report.not_scanned.contains(&"word/theme/theme1.xml".to_string()));
}

#[test]
fn percent_encoded_target_is_not_an_orphan() {
    // Relationship targets are URIs: `image%201.png` names the part
    // `word/media/image 1.png` (OPC part names vs URI escaping).
    let rels = doc_rels(
        r#"<Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image%201.png"/>"#,
    );
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &root_rels()),
        ("word/document.xml", &plain_paragraph()),
        ("word/_rels/document.xml.rels", &rels),
        ("word/media/image 1.png", b"fake-png-bytes"),
    ]);
    assert!(findings_of(&bytes).is_empty());
}

#[test]
fn case_variant_target_is_not_an_orphan() {
    // OPC part names are case-insensitive: `Media/Image1.PNG` reaches
    // `word/media/image1.png`.
    let rels = doc_rels(
        r#"<Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="Media/Image1.PNG"/>"#,
    );
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &root_rels()),
        ("word/document.xml", &plain_paragraph()),
        ("word/_rels/document.xml.rels", &rels),
        ("word/media/image1.png", b"fake-png-bytes"),
    ]);
    assert!(findings_of(&bytes).is_empty());
}

#[test]
fn control_characters_in_values_are_sanitized() {
    // A hostile file can smuggle control characters into a reported value
    // (here via CDATA in `dc:creator`); the report replaces them with
    // U+FFFD so printing the report cannot spoof a terminal.
    let core = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:creator><![CDATA[Quill\u{7}Ramsey]]></dc:creator></cp:coreProperties>";
    let rels = root_rels_with(
        r#"<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>"#,
    );
    let bytes = package(&[
        ("[Content_Types].xml", &content_types()),
        ("_rels/.rels", &rels),
        ("word/document.xml", &plain_paragraph()),
        ("word/_rels/document.xml.rels", &doc_rels("")),
        ("docProps/core.xml", core.as_bytes()),
    ]);
    let findings = findings_of(&bytes);
    assert_eq!(findings.len(), 1);
    assert!(!findings[0].detail.contains('\u{7}'), "raw control char survived: {:?}", findings[0].detail);
    assert!(findings[0].detail.contains('\u{fffd}'));
    assert!(findings[0].detail.contains("Quill"));
}
