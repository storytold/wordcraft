//! A glossary boundary must not reassign its descendants to main.
#![allow(clippy::unwrap_used, clippy::expect_used)]

// This independent regression module reuses the unchanged integration-test fixture builder.
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "../tests/common/mod.rs"]
mod common;

use super::*;
use common::{CT_COMMENTS, CT_FOOTNOTES, CT_GLOSSARY, CT_HEADER, Doc, NS, rels_xml};

const OFFICIAL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/glossaryDocument";
const CUSTOM: &str = "urn:custom/glossaryDocument";
const HEADER: &str = "word/g2/header.xml";

fn rel(kind: &str, target: &str) -> String {
    format!(r#"<Relationship Id="{target}" Type="{kind}" Target="{target}"/>"#)
}

fn role(kind: &str, target: &str) -> String {
    rel(&format!("http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}"), target)
}

fn fixture(nested: Option<&str>, main_to_b: bool, direct_header: bool) -> Doc {
    let mut d = Doc::minimal();
    d.set_document(r#"<w:p><w:r><w:footnoteReference w:id="1"/></w:r></w:p>"#);
    d.add_with_override("word/footnotes.xml", CT_FOOTNOTES, format!(r#"<w:footnotes {NS}><w:footnote w:id="1"><w:p/></w:footnote></w:footnotes>"#));
    let mut main = role("footnotes", "footnotes.xml") + &rel(OFFICIAL, "glossary/document.xml");
    if main_to_b {
        main += &rel(CUSTOM, "g2/document.xml");
    }
    d.set_doc_rels(&main);
    for folder in ["glossary", "g2"] {
        d.add_with_override(&format!("word/{folder}/document.xml"), CT_GLOSSARY, format!(r#"<w:glossaryDocument {NS}><w:docParts><w:docPart><w:docPartBody><w:p><w:r><w:footnoteReference w:id="2"/><w:commentReference w:id="4"/></w:r></w:p></w:docPartBody></w:docPart></w:docParts></w:glossaryDocument>"#));
        d.add_with_override(
            &format!("word/{folder}/footnotes.xml"),
            CT_FOOTNOTES,
            format!(r#"<w:footnotes {NS}><w:footnote w:id="2"><w:p/></w:footnote></w:footnotes>"#),
        );
        d.add_with_override(
            &format!("word/{folder}/comments.xml"),
            CT_COMMENTS,
            format!(r#"<w:comments {NS}><w:comment w:id="4"><w:p/></w:comment></w:comments>"#),
        );
        let mut edges = role("footnotes", "footnotes.xml") + &role("comments", "comments.xml");
        if folder == "glossary" {
            if let Some(kind) = nested {
                edges += &rel(kind, "../g2/document.xml");
            }
            if direct_header {
                edges += &role("header", "../g2/header.xml");
            }
        } else {
            edges += &role("header", "header.xml");
        }
        d.set(&format!("word/{folder}/_rels/document.xml.rels"), rels_xml(&edges));
    }
    d.add_with_override(
        HEADER,
        CT_HEADER,
        format!(r#"<w:hdr {NS}><w:p><w:r><w:footnoteReference w:id="2"/><w:commentReference w:id="4"/></w:r></w:p></w:hdr>"#),
    );
    d
}

fn loaded(d: &Doc) -> Package {
    package::load(&d.build(), &mut Acc::with_limits(Limits::default())).unwrap()
}

fn assert_pass(d: &Doc) {
    let report = validate_bytes(&d.build());
    assert_eq!(report.verdict, Verdict::Pass, "{report:?}");
    assert!(report.findings.is_empty(), "{report:?}");
}

#[test]
fn selected_glossary_keeps_nested_glossary_header_in_both_zip_orders() {
    for direct_header in [false, true] {
        for reverse in [false, true] {
            let mut d = fixture(Some(CUSTOM), false, direct_header);
            if reverse {
                d.entries.reverse();
            }
            assert_pass(&d);
            let mut pkg = loaded(&d);
            for incomplete in [false, true] {
                pkg.entries_truncated = incomplete;
                let map = owner_index_map(&pkg);
                assert_eq!(part_owners(&map, "word/g2/document.xml"), &[OwnerId(2)]);
                assert_eq!(part_owners(&map, HEADER), if direct_header { &[OwnerId(1), OwnerId(2)][..] } else { &[OwnerId(2)][..] });
            }
        }
    }
}

#[test]
fn selected_glossary_traverses_a_second_official_glossary_edge() {
    for reverse in [false, true] {
        let mut d = fixture(Some(OFFICIAL), false, false);
        if reverse {
            d.entries.reverse();
        }
        assert_pass(&d);
        let pkg = loaded(&d);
        assert_eq!(
            (match &pkg.glossary {
                Resolution::One(name) => Some(name.as_str()),
                Resolution::Absent | Resolution::Ambiguous(_) | Resolution::Unknown(_) => None,
            }),
            Some("word/glossary/document.xml")
        );
        assert_eq!(part_owners(&owner_index_map(&pkg), HEADER), &[OwnerId(2)]);
    }
}

#[test]
fn boundary_only_descendants_never_fallback_to_main_or_no_owner() {
    for main_to_b in [false, true] {
        for reverse in [false, true] {
            let mut d = fixture(None, main_to_b, false);
            // A multi-hop chain and cycle must stay out of fallback even when B is orphaned.
            d.set("word/g2/_rels/header.xml.rels", rels_xml(&rel("urn:custom/next", "tail.xml")));
            d.add_with_override("word/g2/tail.xml", CT_HEADER, format!(r#"<w:hdr {NS}><w:p/></w:hdr>"#));
            d.set("word/g2/_rels/tail.xml.rels", rels_xml(&rel("urn:custom/back", "header.xml")));
            d.add_with_override("word/orphan.xml", CT_HEADER, format!(r#"<w:hdr {NS}><w:p/></w:hdr>"#));
            if reverse {
                d.entries.reverse();
            }
            assert_pass(&d);
            let mut pkg = loaded(&d);
            for no_owner in [false, true] {
                if no_owner {
                    pkg.main = Resolution::Absent;
                    pkg.glossary = Resolution::Absent;
                }
                let map = owner_index_map(&pkg);
                let b = owner_scopes(&pkg).iter().position(|o| o.root() == Some("word/g2/document.xml")).unwrap();
                for part in ["word/g2/document.xml", "word/g2/footnotes.xml", "word/g2/comments.xml", HEADER, "word/g2/tail.xml"] {
                    assert_eq!(part_owners(&map, part), &[OwnerId(b)], "{part}: {map:?}");
                }
                assert!(part_owners(&map, "word/orphan.xml").is_empty());
            }
        }
    }
}

#[test]
fn direct_main_reachability_survives_a_boundary_crossed_path() {
    for selected_reaches_b in [false, true] {
        for reverse in [false, true] {
            let mut d = fixture(selected_reaches_b.then_some(CUSTOM), true, false);
            let edges = String::from_utf8(d.get("word/_rels/document.xml.rels").to_vec()).unwrap();
            d.set("word/_rels/document.xml.rels", edges.replace("</Relationships>", &(role("header", "g2/header.xml") + "</Relationships>")));
            if reverse {
                d.entries.reverse();
            }
            let pkg = loaded(&d);
            let map = owner_index_map(&pkg);
            assert_eq!(part_owners(&map, HEADER), &[OwnerId(0), OwnerId(2)]);
            // These IDs really are absent from main: its direct header membership must be checked.
            let report = validate_bytes(&d.build());
            for rule in ["REF-NOTE", "ID-COMMENT-REF"] {
                assert!(report.findings.iter().any(|f| f.rule == rule && f.part.as_deref() == Some(HEADER)), "{report:?}");
            }
        }
    }
}

#[test]
fn nested_glossary_header_uses_selected_glossary_definitions() {
    for reverse in [false, true] {
        let mut d = fixture(Some(CUSTOM), false, false);
        // B still defines 2/4 and its header uses B's own selected sources.
        // Keep the references only in the header: findings deduplicate the same missing ID.
        for folder in ["glossary", "g2"] {
            let name = format!("word/{folder}/document.xml");
            let xml = String::from_utf8(d.get(&name).to_vec()).unwrap();
            d.set(&name, xml.replace(r#"<w:footnoteReference w:id="2"/><w:commentReference w:id="4"/>"#, ""));
        }
        d.set("word/glossary/footnotes.xml", format!(r#"<w:footnotes {NS}><w:footnote w:id="8"><w:p/></w:footnote></w:footnotes>"#));
        d.set("word/glossary/comments.xml", format!(r#"<w:comments {NS}><w:comment w:id="9"><w:p/></w:comment></w:comments>"#));
        if reverse {
            d.entries.reverse();
        }
        assert_eq!(part_owners(&owner_index_map(&loaded(&d)), HEADER), &[OwnerId(2)]);
        let report = validate_bytes(&d.build());
        for rule in ["REF-NOTE", "ID-COMMENT-REF"] {
            assert!(!report.findings.iter().any(|f| f.rule == rule && f.part.as_deref() == Some(HEADER)), "{report:?}");
        }
    }
}

#[test]
fn no_main_selected_glossary_still_traverses_nested_glossaries_as_owner_zero() {
    for reverse in [false, true] {
        let mut d = fixture(Some(CUSTOM), false, false);
        d.remove("_rels/.rels");
        d.remove("word/document.xml");
        // The selected glossary is supplied explicitly to isolate owner-zero traversal from
        // the independently ambiguous no-main content-type selection.
        if reverse {
            d.entries.reverse();
        }
        let mut pkg = loaded(&d);
        pkg.glossary = Resolution::One("word/glossary/document.xml".to_string());
        assert!(!matches!(pkg.main, Resolution::One(_)));
        let map = owner_index_map(&pkg);
        assert_eq!(part_owners(&map, "word/g2/document.xml"), &[OwnerId(1)]);
        assert_eq!(part_owners(&map, HEADER), &[OwnerId(1)]);
    }
}
