//! Glossary selection and ownership remain independent of ZIP order.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[allow(dead_code)]
#[path = "../tests/common/mod.rs"]
mod common;

use super::*;
use common::{CT_COMMENTS, CT_FOOTNOTES, CT_GLOSSARY, Doc, NS, rels_xml};

const OFFICIAL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/glossaryDocument";
const STRICT: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships/glossaryDocument";

fn rel(kind: &str, target: &str) -> String {
    format!(r#"<Relationship Id="{target}" Type="{kind}" Target="{target}"/>"#)
}

fn role_rel(kind: &str, target: &str) -> String {
    rel(&format!("http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}"), target)
}

fn add_glossary(d: &mut Doc, folder: &str, note: u8, comment: u8) {
    d.add_with_override(&format!("word/{folder}/document.xml"), CT_GLOSSARY, format!(r#"<w:glossaryDocument {NS}><w:docParts><w:docPart><w:docPartBody><w:p><w:r><w:footnoteReference w:id="{note}"/><w:commentReference w:id="{comment}"/></w:r></w:p></w:docPartBody></w:docPart></w:docParts></w:glossaryDocument>"#));
    d.add_with_override(
        &format!("word/{folder}/footnotes.xml"),
        CT_FOOTNOTES,
        format!(r#"<w:footnotes {NS}><w:footnote w:id="{note}"><w:p/></w:footnote></w:footnotes>"#),
    );
    d.add_with_override(
        &format!("word/{folder}/comments.xml"),
        CT_COMMENTS,
        format!(r#"<w:comments {NS}><w:comment w:id="{comment}"><w:p/></w:comment></w:comments>"#),
    );
    d.set(
        &format!("word/{folder}/_rels/document.xml.rels"),
        rels_xml(&(role_rel("footnotes", "footnotes.xml") + &role_rel("comments", "comments.xml"))),
    );
}

fn fixture(a: Option<&str>, b: Option<&str>, reverse: bool, second: bool) -> Doc {
    let mut d = Doc::minimal();
    d.set_document(r#"<w:p><w:r><w:footnoteReference w:id="1"/></w:r></w:p>"#);
    d.add_with_override("word/footnotes.xml", CT_FOOTNOTES, format!(r#"<w:footnotes {NS}><w:footnote w:id="1"><w:p/></w:footnote></w:footnotes>"#));
    let mut rels = role_rel("footnotes", "footnotes.xml");
    for (kind, target) in [(a, "glossary/document.xml"), (b, "g2/document.xml")] {
        if let Some(kind) = kind {
            rels += &rel(kind, target);
        }
    }
    d.set_doc_rels(&rels);
    add_glossary(&mut d, "glossary", 2, 4);
    if second {
        add_glossary(&mut d, "g2", 3, 5);
    }
    if reverse {
        d.entries.reverse();
    }
    d
}

fn loaded(d: &Doc) -> Package {
    package::load(&d.build(), &mut Acc::with_limits(Limits::default())).unwrap()
}

fn no_false_joins(report: &Report) {
    assert!(!report.findings.iter().any(|f| matches!(f.rule, "REF-NOTE" | "ID-COMMENT-REF")), "{report:?}");
}

fn ambiguous(report: &Report) {
    no_false_joins(report);
    assert_eq!(report.verdict, Verdict::Incomplete, "{report:?}");
    assert!(report.coverage_notes.iter().any(|n| n.contains("ambiguous glossaryDocument")), "{report:?}");
    assert!(!report.checks_run.contains(&"REF-NOTE"), "{report:?}");
}

#[test]
fn mb1_strict_glossary_plus_orphan_is_ambiguous_in_both_zip_orders() {
    for reverse in [false, true] {
        ambiguous(&validate_bytes(&fixture(Some(STRICT), None, reverse, true).build()));
    }
}

#[test]
fn mb2_official_and_strict_glossaries_pass_in_both_zip_orders() {
    for reverse in [false, true] {
        let report = validate_bytes(&fixture(Some(OFFICIAL), Some(STRICT), reverse, true).build());
        no_false_joins(&report);
        assert_eq!(report.verdict, Verdict::Pass, "{report:?}");
    }
}

#[test]
fn mb3_two_strict_glossaries_are_ambiguous_in_both_zip_orders() {
    for reverse in [false, true] {
        let d = fixture(Some(STRICT), Some(STRICT), reverse, true);
        ambiguous(&validate_bytes(&d.build()));
        let pkg = loaded(&d);
        assert!(!matches!(pkg.glossary, Resolution::One(_)));
        assert!(pkg.roots_ambiguous);
    }
}

#[test]
fn official_glossary_isolates_custom_and_orphan_glossaries_in_both_zip_orders() {
    for kind in [Some("urn:custom/glossaryDocument"), None] {
        for reverse in [false, true] {
            let report = validate_bytes(&fixture(Some(OFFICIAL), kind, reverse, true).build());
            no_false_joins(&report);
            assert_eq!(report.verdict, Verdict::Pass, "{report:?}");
        }
    }
}

#[test]
fn glossary_typed_parts_are_excluded_from_main_traversal_and_fallback() {
    for kind in [Some(STRICT), Some("urn:custom/glossaryDocument"), None] {
        for reverse in [false, true] {
            let mut pkg = loaded(&fixture(Some(OFFICIAL), kind, reverse, true));
            for incomplete in [false, true] {
                pkg.entries_truncated = incomplete;
                let map = owner_index_map(&pkg);
                assert_eq!(part_owners(&map, "word/glossary/document.xml"), &[OwnerId(1)]);
                assert_eq!(part_owners(&map, "word/g2/document.xml"), &[OwnerId(2)], "{map:?}");
                if incomplete {
                    // With fallback disabled, these memberships expose traversal through B itself.
                    assert_eq!(part_owners(&map, "word/g2/footnotes.xml"), &[OwnerId(2)], "{map:?}");
                    assert_eq!(part_owners(&map, "word/g2/comments.xml"), &[OwnerId(2)], "{map:?}");
                }
            }
        }
    }
}

#[test]
fn single_glossary_fallback_and_official_selection_preserve_m1_m2() {
    for kind in [Some(OFFICIAL), Some(STRICT), None] {
        for reverse in [false, true] {
            let d = fixture(kind, None, reverse, false);
            let report = validate_bytes(&d.build());
            assert_eq!(report.verdict, Verdict::Pass, "{report:?}");
            let pkg = loaded(&d);
            assert_eq!(
                (match &pkg.glossary {
                    Resolution::One(name) => Some(name.as_str()),
                    Resolution::Absent | Resolution::Ambiguous(_) | Resolution::Unknown(_) => None,
                }),
                Some("word/glossary/document.xml")
            );
            assert_eq!(part_owners(&owner_index_map(&pkg), "word/glossary/document.xml"), &[OwnerId(1)]);
        }
    }
}

#[test]
fn official_competing_glossaries_remain_ambiguous_in_both_zip_orders() {
    for reverse in [false, true] {
        ambiguous(&validate_bytes(&fixture(Some(OFFICIAL), Some(OFFICIAL), reverse, true).build()));
    }
}

#[test]
fn no_main_single_glossary_fallback_remains_the_primary_glossary_owner() {
    for reverse in [false, true] {
        let mut d = fixture(None, None, reverse, false);
        d.remove("_rels/.rels");
        d.remove("word/document.xml");
        let pkg = loaded(&d);
        assert!(!matches!(pkg.main, Resolution::One(_)));
        assert_eq!(
            (match &pkg.glossary {
                Resolution::One(name) => Some(name.as_str()),
                Resolution::Absent | Resolution::Ambiguous(_) | Resolution::Unknown(_) => None,
            }),
            Some("word/glossary/document.xml")
        );
        let map = owner_index_map(&pkg);
        assert_eq!(part_owners(&map, "word/glossary/document.xml"), &[OwnerId(0)]);
        assert_eq!(part_owners(&map, "word/glossary/footnotes.xml"), &[OwnerId(0)]);
        let report = validate_bytes(&d.build());
        no_false_joins(&report);
        assert!(report.findings.iter().any(|f| f.rule == "PKG-NO-MAIN"), "{report:?}");
        // Its actual root is still checked even though content type alone discovered the owner.
        d.set("word/glossary/document.xml", "<foreign/>");
        let report = validate_bytes(&d.build());
        assert!(report.coverage_notes.iter().any(|n| n.contains("word/glossary/document.xml has root {}foreign")), "{report:?}");
        assert!(!report.checks_run.contains(&"REF-NOTE"), "{report:?}");
    }
}

#[test]
fn no_main_multiple_fallback_glossaries_have_no_owner_anchor() {
    for reverse in [false, true] {
        let mut d = fixture(None, None, reverse, true);
        d.remove("_rels/.rels");
        d.remove("word/document.xml");
        let pkg = loaded(&d);
        assert!(!matches!(pkg.main, Resolution::One(_)));
        assert!(!matches!(pkg.glossary, Resolution::One(_)));
        assert!(pkg.roots_ambiguous);
        let map = owner_index_map(&pkg);
        for part in ["word/glossary/document.xml", "word/g2/document.xml"] {
            let index = owner_scopes(&pkg).iter().position(|o| o.root() == Some(part)).unwrap();
            assert_eq!(part_owners(&map, part), &[OwnerId(index)], "{map:?}");
        }
        let report = validate_bytes(&d.build());
        no_false_joins(&report);
        assert!(report.coverage_notes.iter().any(|n| n.contains("ambiguous glossaryDocument")), "{report:?}");
        // Genuine no-main errors retain precedence over the new ambiguity coverage.
        assert_eq!(report.verdict, Verdict::Fail, "{report:?}");
        assert!(report.findings.iter().any(|f| f.rule == "PKG-NO-MAIN"), "{report:?}");
        assert!(!report.checks_run.contains(&"REF-NOTE"), "{report:?}");
    }
}
