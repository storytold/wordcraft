//! Whole-surface review controls: owner roots, singleton loss and negative evidence.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[allow(dead_code)]
mod common;

use common::{CT_COMMENTS, CT_CUSTOM_PROPS, CT_GLOSSARY, CT_HEADER, Doc, NS, package_rels_xml, rels_xml};
use wordcraft_validate::{Report, Verdict, validate_bytes};

fn rel(kind: &str, target: &str) -> String {
    format!(r#"<Relationship Id="{target}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}" Target="{target}"/>"#)
}

fn glossary(body: &str) -> String {
    format!(r#"<w:glossaryDocument {NS}><w:docParts><w:docPart><w:docPartBody>{body}</w:docPartBody></w:docPart></w:docParts></w:glossaryDocument>"#)
}

/// Compare the entire serialized report, including coverage and checks, across ZIP order.
fn both_orders(mut doc: Doc, check: impl Fn(&Report)) {
    let forward = validate_bytes(&doc.build());
    check(&forward);
    doc.entries.reverse();
    let reverse = validate_bytes(&doc.build());
    check(&reverse);
    assert_eq!(serde_json::to_vec(&forward).unwrap(), serde_json::to_vec(&reverse).unwrap());
}

fn nested_glossary(root: &str, direct: bool) -> Doc {
    let mut d = Doc::minimal();
    d.add_with_override("word/a.xml", CT_GLOSSARY, glossary("<w:p/>"));
    d.add_with_override("word/b.xml", CT_GLOSSARY, root);
    d.set_doc_rels(&rel("glossaryDocument", if direct { "b.xml" } else { "a.xml" }));
    if !direct {
        d.set("word/_rels/a.xml.rels", rels_xml(&rel("glossaryDocument", "b.xml")));
    }
    d
}

fn unsupported_glossary(root: &str) {
    for direct in [false, true] {
        both_orders(nested_glossary(root, direct), |r| {
            assert_eq!(r.verdict, Verdict::Incomplete, "{r:?}");
            assert!(r.findings.is_empty(), "{r:?}");
            assert!(r.coverage_notes.iter().any(|n| n.contains("word/b.xml") && n.contains("root")), "{r:?}");
        });
    }
}

#[test]
fn every_glossary_owner_gates_strict_roots() {
    unsupported_glossary(
        &glossary(r#"<w:p><w:r><w:footnoteReference w:id="999"/></w:r></w:p>"#)
            .replace("http://schemas.openxmlformats.org/wordprocessingml/2006/main", "http://purl.oclc.org/ooxml/wordprocessingml/main"),
    );
}

#[test]
fn every_glossary_owner_gates_foreign_roots() {
    unsupported_glossary("<foreign/>");
}

#[test]
fn nested_transitional_glossary_retains_genuine_missing_note() {
    for direct in [false, true] {
        both_orders(nested_glossary(&glossary(r#"<w:p><w:r><w:footnoteReference w:id="999"/></w:r></w:p>"#), direct), |r| {
            assert_eq!(r.verdict, Verdict::Fail, "{r:?}");
            assert!(r.findings.iter().any(|f| f.rule == "REF-NOTE" && f.part.as_deref() == Some("word/b.xml")), "{r:?}");
        });
    }
}

fn custom_properties(competing: bool) -> Doc {
    let mut d = Doc::minimal();
    let xml = r#"<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/custom-properties"><property pid="2" name="Same"/><property pid="3" name="same"/></Properties>"#;
    d.add_with_override("docProps/custom.xml", CT_CUSTOM_PROPS, xml);
    let mut root_rels = String::from_utf8(package_rels_xml()).unwrap();
    root_rels = root_rels.replace("</Relationships>", &(rel("custom-properties", "docProps/custom.xml") + "</Relationships>"));
    d.set("_rels/.rels", root_rels);
    if competing {
        d.add_with_override("docProps/unrelated.xml", CT_CUSTOM_PROPS, xml);
    }
    d
}

#[test]
fn ambiguous_custom_properties_cannot_claim_duplicate_check_ran() {
    both_orders(custom_properties(true), |r| {
        assert_eq!(r.verdict, Verdict::Incomplete, "{r:?}");
        assert!(r.findings.is_empty(), "{r:?}");
        assert!(!r.checks_run.contains(&"PROP-DUP-NAME"), "{r:?}");
        assert!(r.coverage_notes.iter().any(|n| n.contains("PROP-DUP-NAME") && n.contains("ambiguous")), "{r:?}");
    });
}

#[test]
fn unique_custom_properties_keep_duplicate_error() {
    both_orders(custom_properties(false), |r| {
        assert_eq!(r.verdict, Verdict::Fail, "{r:?}");
        assert!(r.findings.iter().any(|f| f.rule == "PROP-DUP-NAME"), "{r:?}");
        assert!(r.checks_run.contains(&"PROP-DUP-NAME"), "{r:?}");
    });
}

#[test]
fn absent_custom_properties_are_complete_and_inapplicable() {
    both_orders(Doc::minimal(), |r| {
        assert_eq!(r.verdict, Verdict::Pass, "{r:?}");
        assert!(r.coverage_notes.is_empty(), "{r:?}");
        assert!(r.checks_run.contains(&"PROP-DUP-NAME"), "{r:?}");
    });
}

#[test]
fn unknown_custom_properties_do_not_claim_duplicate_check_ran() {
    let mut d = Doc::minimal();
    // Discovery cannot prove the unexamined manifest suffix contains no candidate.
    let manifest = String::from_utf8(d.get("[Content_Types].xml").to_vec()).unwrap();
    d.set("[Content_Types].xml", manifest.replace("</Types>", "<"));
    both_orders(d, |r| {
        assert!(!r.checks_run.contains(&"PROP-DUP-NAME"), "{r:?}");
        assert!(r.coverage_notes.iter().any(|n| n.contains("PROP-DUP-NAME") && n.contains("unknown")), "{r:?}");
        assert!(!r.findings.iter().any(|f| f.rule == "PROP-DUP-NAME"), "{r:?}");
    });
}

/// Main establishes membership of comments before fanout exhausts the shared graph budget.
fn graph_fanout(fanout: usize, genuine: bool) -> Doc {
    let mut d = Doc::minimal();
    if genuine {
        d.set_document(r#"<w:p><w:r><w:footnoteReference w:id="999"/></w:r></w:p>"#);
    }
    d.add_with_override("word/a.xml", CT_GLOSSARY, glossary(r#"<w:p><w:r><w:commentReference w:id="7"/></w:r></w:p>"#));
    d.add_with_override("word/comments.xml", CT_COMMENTS, format!(r#"<w:comments {NS}><w:comment w:id="7"><w:p/></w:comment></w:comments>"#));
    d.add_with_override("word/empty.xml", CT_HEADER, format!(r#"<w:hdr {NS}><w:p/></w:hdr>"#));
    d.set("word/_rels/a.xml.rels", rels_xml(&rel("comments", "comments.xml")));
    let mut main = rel("glossaryDocument", "a.xml");
    let fanout_rels: String = (0..18_000).map(|i| format!(r#"<Relationship Id="r{i}" Type="urn:custom" Target="empty.xml"/>"#)).collect();
    for i in 0..fanout {
        d.set(&format!("word/fanout{i}.xml"), "<x/>");
        d.set(&format!("word/_rels/fanout{i}.xml.rels"), rels_xml(&fanout_rels));
        main += &rel("urn:custom", &format!("fanout{i}.xml"));
    }
    main += &rel("comments", "comments.xml");
    d.set_doc_rels(&main);
    d
}

#[test]
fn incomplete_graph_memberships_cannot_prove_unused_comment() {
    both_orders(graph_fanout(6, false), |r| {
        assert_eq!(r.verdict, Verdict::Incomplete, "{r:?}");
        assert!(r.findings.is_empty(), "{r:?}");
        assert!(r.coverage_notes.iter().any(|n| n.contains("graph") && n.contains("budget")), "{r:?}");
        assert!(!r.coverage_notes.iter().any(|n| n.contains("never referenced")), "{r:?}");
    });
}

#[test]
fn complete_graph_keeps_shared_comment_reference() {
    both_orders(graph_fanout(5, false), |r| {
        assert_eq!(r.verdict, Verdict::Pass, "{r:?}");
        assert!(r.findings.is_empty(), "{r:?}");
        assert!(r.coverage_notes.is_empty(), "{r:?}");
    });
}

#[test]
fn incomplete_graph_keeps_independent_missing_note() {
    both_orders(graph_fanout(6, true), |r| {
        assert_eq!(r.verdict, Verdict::Fail, "{r:?}");
        assert!(r.findings.iter().any(|f| f.rule == "REF-NOTE" && f.part.as_deref() == Some("word/document.xml")), "{r:?}");
        assert!(!r.coverage_notes.iter().any(|n| n.contains("never referenced")), "{r:?}");
    });
}

fn unowned_header(reached: bool) -> Doc {
    let mut d = Doc::minimal();
    d.add_with_override("word/orphan.xml", CT_HEADER, format!(r#"<w:hdr {NS}><w:p><w:pPr><w:pStyle w:val="absent"/><w:numPr><w:numId w:val="9"/></w:numPr></w:pPr><w:r><w:footnoteReference w:id="9"/><w:commentReference w:id="9"/></w:r></w:p></w:hdr>"#));
    if reached {
        d.set_doc_rels(&rel("header", "orphan.xml"));
    }
    d
}

#[test]
fn established_unowned_references_have_out_of_scope_coverage() {
    both_orders(unowned_header(false), |r| {
        assert_eq!(r.verdict, Verdict::Pass, "{r:?}");
        assert!(r.findings.is_empty(), "{r:?}");
        assert_eq!(r.coverage_notes.iter().filter(|n| n.contains("out of scope") && n.contains("1 established-unowned")).count(), 1, "{r:?}");
    });
}

#[test]
fn reached_references_remain_in_scope() {
    both_orders(unowned_header(true), |r| {
        assert_eq!(r.verdict, Verdict::Fail, "{r:?}");
        assert!(r.findings.iter().any(|f| f.rule == "REF-NOTE"), "{r:?}");
        assert!(!r.coverage_notes.iter().any(|n| n.contains("out of scope")), "{r:?}");
    });
}
