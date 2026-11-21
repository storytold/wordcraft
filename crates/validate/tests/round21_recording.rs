//! Narrowed F2: unreadable selected document roots cannot claim vacuous reference joins.
#![allow(dead_code)]
mod common;
use common::*;
use wordcraft_validate::{Report, Verdict, validate_bytes};

const NOTE: &str = r#"<w:p><w:r><w:footnoteReference w:id="5"/></w:r></w:p>"#;
const RID: &str = r#"<w:p><w:hyperlink r:id="last"><w:r><w:t>link</w:t></w:r></w:hyperlink></w:p>"#;
const COMMENT: &str = r#"<w:p><w:r><w:commentReference w:id="1"/></w:r></w:p>"#;
const JOINS: &[&str] = &["REF-NOTE", "ID-COMMENT-REF", "REL-REF-MISSING"];
const GLOSSARY: &str = "word/glossary/document.xml";

fn edge(id: &str, kind: &str, target: &str) -> String {
    format!(r#"<Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}" Target="{target}"/>"#)
}
fn notes(d: &mut Doc, prefix: &str, id: u32) {
    d.add_with_override(
        &format!("{prefix}/footnotes.xml"),
        CT_FOOTNOTES,
        format!(r#"<w:footnotes {NS}><w:footnote w:id="{id}"><w:p/></w:footnote></w:footnotes>"#),
    );
}
fn glossary(d: &mut Doc, body: &str) {
    d.add_with_override(
        GLOSSARY,
        CT_GLOSSARY,
        format!(r#"<w:glossaryDocument {NS}><w:docParts><w:docPart><w:docPartPr><w:name w:val="g"/></w:docPartPr><w:docPartBody>{body}</w:docPartBody></w:docPart></w:docParts></w:glossaryDocument>"#),
    );
}
fn orders(d: Doc, root: &str, assert_report: impl Fn(&Report)) {
    for reverse in [false, true] {
        let mut d = Doc { entries: d.entries.clone() };
        if reverse {
            d.entries.reverse();
        }
        let report = validate_bytes(&corrupt_entry_data(&d.build(), root));
        assert_eq!(report.verdict, Verdict::Incomplete, "reverse={reverse}: {report:?}");
        assert!(report.findings.is_empty(), "reverse={reverse}: {report:?}");
        assert!(report.coverage_notes.iter().any(|n| n.contains(root)), "{report:?}");
        assert_report(&report);
    }
}
fn skipped(r: &Report) {
    for rule in JOINS {
        assert!(!r.checks_run.contains(rule), "overclaimed {rule}: {r:?}");
    }
}
fn readable_control(d: &Doc, rule: &str, part: &str) {
    let r = validate_bytes(&d.build());
    assert_eq!(r.verdict, Verdict::Fail, "{r:?}");
    assert_eq!(r.findings.len(), 1, "{r:?}");
    assert_eq!(r.findings[0].rule, rule, "{r:?}");
    assert_eq!(r.findings[0].part.as_deref(), Some(part), "{r:?}");
    assert!(r.checks_run.contains(&rule), "{r:?}");
}

#[test]
fn corrupt_main_document_notes_skip_all_three_joins_both_orders() {
    for with_notes in [false, true] {
        let mut d = Doc::minimal();
        d.set_document(NOTE);
        if with_notes {
            notes(&mut d, "word", 1);
            d.set_doc_rels(&edge("fn", "footnotes", "footnotes.xml"));
        }
        readable_control(&d, "REF-NOTE", "word/document.xml");
        orders(d, "word/document.xml", skipped);
    }
}
#[test]
fn corrupt_main_document_relationship_refs_skip_all_three_joins_both_orders() {
    let mut d = Doc::minimal();
    d.set_document(RID);
    readable_control(&d, "REL-REF-MISSING", "word/document.xml");
    orders(d, "word/document.xml", skipped);
}
#[test]
fn corrupt_glossary_document_notes_are_not_laundered_by_vacuous_main_both_orders() {
    for with_notes in [false, true] {
        let mut d = Doc::minimal();
        // The main owner is vacuous, including its source-local relationship scope.
        d.remove("word/_rels/document.xml.rels");
        glossary(&mut d, NOTE);
        if with_notes {
            notes(&mut d, "word/glossary", 1);
            d.set("word/glossary/_rels/document.xml.rels", rels_xml(&edge("fn", "footnotes", "footnotes.xml")));
        }
        readable_control(&d, "REF-NOTE", GLOSSARY);
        orders(d, GLOSSARY, skipped);
    }
}
#[test]
fn corrupt_glossary_document_relationship_refs_are_not_laundered_by_vacuous_main_both_orders() {
    let mut d = Doc::minimal();
    d.remove("word/_rels/document.xml.rels");
    glossary(&mut d, RID);
    readable_control(&d, "REL-REF-MISSING", GLOSSARY);
    orders(d, GLOSSARY, skipped);
}
#[test]
fn active_glossary_joins_survive_unreadable_main_document_both_orders() {
    let mut d = Doc::minimal();
    d.set_document(NOTE);
    glossary(&mut d, &(NOTE.to_string() + RID + COMMENT));
    notes(&mut d, "word/glossary", 5);
    d.add_with_override(
        "word/glossary/comments.xml",
        CT_COMMENTS,
        format!(r#"<w:comments {NS}><w:comment w:id="1"><w:p/></w:comment></w:comments>"#),
    );
    d.set(
        "word/glossary/_rels/document.xml.rels",
        rels_xml(&(edge("fn", "footnotes", "footnotes.xml") + &edge("c", "comments", "comments.xml")
            + r#"<Relationship Id="last" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.invalid/" TargetMode="External"/>"#)),
    );
    orders(d, "word/document.xml", |r| {
        for rule in JOINS {
            assert!(r.checks_run.contains(rule), "lost active glossary {rule}: {r:?}");
        }
    });
}

#[test]
fn undecodable_main_and_glossary_documents_skip_reference_joins_both_orders() {
    for glossary_root in [false, true] {
        let mut d = Doc::minimal();
        d.remove("word/_rels/document.xml.rels");
        let root = if glossary_root {
            glossary(&mut d, NOTE);
            GLOSSARY
        } else {
            "word/document.xml"
        };
        let xml = String::from_utf8(d.get(root).to_vec()).unwrap();
        // Exercise strict text decoding, independently of the corrupt-ZIP read path above.
        d.set(root, format!("<?xml version=\"1.0\" encoding=\"X-R21-UNSUPPORTED\"?>{}", &xml[xml.find("?>").map_or(0, |i| i + 2)..]));
        for reverse in [false, true] {
            let mut d = Doc { entries: d.entries.clone() };
            if reverse {
                d.entries.reverse();
            }
            let r = validate_bytes(&d.build());
            assert_eq!(r.verdict, Verdict::Incomplete, "{r:?}");
            assert!(r.findings.is_empty(), "{r:?}");
            assert!(r.coverage_notes.iter().any(|n| n.contains(root) && n.contains("unsupported encoding")), "{r:?}");
            skipped(&r);
        }
    }
}
