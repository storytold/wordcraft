//! Supporting joins use an owner's own relationships, and owner roots gate the scanner.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[allow(dead_code)]
mod common;

use common::{CT_COMMENTS, CT_FOOTNOTES, CT_GLOSSARY, CT_HEADER, CT_SETTINGS, Doc, NS, rels_xml};
use wordcraft_validate::{Verdict, validate_bytes};

fn rel(kind: &str, target: &str) -> String {
    rel_uri(&format!("http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}"), target)
}

fn rel_uri(kind: &str, target: &str) -> String {
    format!(r#"<Relationship Id="{target}" Type="{kind}" Target="{target}"/>"#)
}

fn note_fixture(shape: &str) -> Doc {
    let mut d = Doc::minimal();
    d.add_with_override(
        "word/footnotes.xml",
        CT_FOOTNOTES,
        format!(r#"<w:footnotes {NS}><w:footnote w:type="separator" w:id="-1"><w:p/></w:footnote></w:footnotes>"#),
    );
    let mut main = rel("footnotes", "footnotes.xml");
    let empty = format!(r#"<w:settings {NS}/>"#);
    let listing = format!(r#"<w:settings {NS}><w:footnotePr><w:footnote w:id="-1"/></w:footnotePr></w:settings>"#);
    match shape {
        "C0" | "C1" => {
            d.add_with_override("word/settings.xml", CT_SETTINGS, empty);
            main += &rel("settings", "settings.xml");
            if shape == "C1" {
                d.add_with_override("word/orphan.xml", CT_SETTINGS, listing);
            }
        }
        "C2" => d.add_with_override("word/orphan.xml", CT_SETTINGS, listing),
        "H1" => {
            d.add_with_override("word/settings.xml", CT_SETTINGS, listing);
            main += &rel_uri("urn:custom/settings", "settings.xml");
        }
        "H2" | "H3" | "H4" => {
            d.add_with_override("word/settings.xml", CT_SETTINGS, listing);
            let source = if shape == "H3" {
                "footnotes.xml"
            } else {
                d.add_with_override("word/header1.xml", CT_HEADER, format!(r#"<w:hdr {NS}><w:p/></w:hdr>"#));
                main += &rel("header", "header1.xml");
                "header1.xml"
            };
            d.set(&format!("word/_rels/{source}.rels"), rels_xml(&rel("settings", "settings.xml")));
            if shape == "H4" {
                d.add_with_override("word/settings2.xml", CT_SETTINGS, empty);
                main += &rel("settings", "settings2.xml");
            }
        }
        "D2" => {
            d.add_with_override("word/settings.xml", CT_SETTINGS, listing);
            main += &rel("settings", "settings.xml");
            d.add_with_override(
                "word/orphan.xml",
                CT_FOOTNOTES,
                format!(r#"<w:footnotes {NS}><w:footnote w:type="continuationSeparator" w:id="0"><w:p/></w:footnote></w:footnotes>"#),
            );
        }
        _ => panic!("unknown fixture"),
    }
    d.set_doc_rels(&main);
    d
}

#[test]
fn note_joins_ignore_orphan_custom_and_descendant_sources() {
    let mut failures = Vec::new();
    for shape in ["C0", "C1", "C2", "H1", "H2", "H3", "H4", "D2"] {
        let report = validate_bytes(&note_fixture(shape).build());
        eprintln!("fixture {shape}: {}", serde_json::to_string(&report).unwrap());
        let expected = if shape == "D2" { Verdict::Pass } else { Verdict::Fail };
        let listing = report.findings.iter().any(|f| f.rule == "NOTE-PR-LISTING");
        if report.verdict != expected || listing != (shape != "D2") {
            failures.push(shape);
        }
    }
    assert!(failures.is_empty(), "wrong note-join results: {failures:?}");
}

#[test]
fn main_root_must_be_transitional_document() {
    let mut failures = Vec::new();
    for (shape, ns, root) in [
        ("G1", "http://purl.oclc.org/ooxml/wordprocessingml/main", "document"),
        ("G1c", "http://schemas.openxmlformats.org/wordprocessingml/2006/main", "document"),
        ("G2", "urn:x", "document"),
        ("G3", "http://schemas.openxmlformats.org/wordprocessingml/2006/main", "footnotes"),
    ] {
        let mut d = Doc::minimal();
        let rns = if shape == "G1" {
            "http://purl.oclc.org/ooxml/officeDocument/relationships"
        } else {
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
        };
        d.set_document_raw(&format!(r#"<w:{root} xmlns:w="{ns}" xmlns:r="{rns}"><w:body><w:p><w:r><w:footnoteReference w:id="99"/></w:r><w:hyperlink r:id="rNope"><w:r><w:t>link</w:t></w:r></w:hyperlink></w:p></w:body></w:{root}>"#));
        if matches!(shape, "G2" | "G3") {
            d.set_document_raw(&format!(r#"<w:{root} xmlns:w="{ns}"/>"#));
        }
        let report = validate_bytes(&d.build());
        eprintln!("fixture {shape}: {}", serde_json::to_string(&report).unwrap());
        let correct = if shape == "G1c" {
            report.verdict == Verdict::Fail && ["REF-NOTE", "REL-REF-MISSING"].iter().all(|rule| report.findings.iter().any(|f| f.rule == *rule))
        } else {
            report.verdict == Verdict::Incomplete
                && report.coverage_notes.iter().any(|n| n.contains(if shape == "G1" { "Strict" } else { root }))
                && !report.checks_run.contains(&"REF-NOTE")
        };
        if !correct {
            failures.push(shape);
        }
    }
    assert!(failures.is_empty(), "wrong root-gate results: {failures:?}");
}

fn glossary_fixture(kind: &str) -> Doc {
    let mut d = Doc::minimal();
    d.add_with_override("word/glossary/document.xml", CT_GLOSSARY, format!(r#"<w:glossaryDocument {NS}><w:docParts><w:docPart><w:docPartBody><w:p><w:r><w:footnoteReference w:id="2"/><w:commentReference w:id="4"/></w:r></w:p></w:docPartBody></w:docPart></w:docParts></w:glossaryDocument>"#));
    d.add_with_override(
        "word/glossary/footnotes.xml",
        CT_FOOTNOTES,
        format!(r#"<w:footnotes {NS}><w:footnote w:id="2"><w:p/></w:footnote></w:footnotes>"#),
    );
    d.add_with_override(
        "word/glossary/comments.xml",
        CT_COMMENTS,
        format!(r#"<w:comments {NS}><w:comment w:id="4"><w:p/></w:comment></w:comments>"#),
    );
    d.set("word/glossary/_rels/document.xml.rels", rels_xml(&(rel("footnotes", "footnotes.xml") + &rel("comments", "comments.xml"))));
    d.set_doc_rels(&rel_uri(kind, "glossary/document.xml"));
    d
}

#[test]
fn glossary_root_is_an_owner_boundary_for_every_relationship_type() {
    let mut failures = Vec::new();
    for (shape, kind) in [
        ("M1", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/glossaryDocument"),
        ("M2", "http://purl.oclc.org/ooxml/officeDocument/relationships/glossaryDocument"),
    ] {
        let report = validate_bytes(&glossary_fixture(kind).build());
        eprintln!("fixture {shape}: {}", serde_json::to_string(&report).unwrap());
        if report.verdict != Verdict::Pass {
            failures.push(shape);
        }
    }
    assert!(failures.is_empty(), "wrong glossary results: {failures:?}");
}
