//! Round 20 N1 completion probes, including the explicitly retained recording residuals.
#![allow(dead_code)]
mod common;
use common::*;
use wordcraft_validate::{Report, Verdict, validate_bytes};

const NOTE: &str = r#"<w:p><w:r><w:footnoteReference w:id="1"/></w:r></w:p>"#;
const RID: &str = r#"<w:p><w:hyperlink r:id="last"><w:r><w:t>link</w:t></w:r></w:hyperlink></w:p>"#;
fn edge(id: &str, kind: &str, target: &str) -> String {
    format!(r#"<Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}" Target="{target}"/>"#)
}
fn notes() -> Doc {
    let mut d = Doc::minimal();
    d.set_document(NOTE);
    d.add_with_override("word/footnotes.xml", CT_FOOTNOTES, format!(r#"<w:footnotes {NS}><w:footnote w:id="1"><w:p/></w:footnote></w:footnotes>"#));
    d.set_doc_rels(&edge("fn", "footnotes", "footnotes.xml"));
    d
}
fn depth_bomb(tail: &str) -> String {
    "<w:sdt><w:sdtContent>".repeat(300) + tail + &"</w:sdtContent></w:sdt>".repeat(300)
}
fn exact(report: &Report, verdict: Verdict, findings: &[(&str, &str)], yes: &[&str], no: &[&str]) {
    assert_eq!(report.verdict, verdict, "{report:?}");
    let mut got: Vec<_> = report.findings.iter().map(|f| (f.rule, f.part.as_deref().unwrap_or(""))).collect();
    got.sort_unstable();
    let mut expected = findings.to_vec();
    expected.sort_unstable();
    assert_eq!(got, expected, "{report:?}");
    for rule in yes {
        assert!(report.checks_run.contains(rule), "missing {rule}: {report:?}");
    }
    for rule in no {
        assert!(!report.checks_run.contains(rule), "overclaimed {rule}: {report:?}");
    }
    let unique: std::collections::HashSet<_> = report.checks_run.iter().collect();
    assert_eq!(unique.len(), report.checks_run.len(), "a rule was recorded twice");
}
fn orders(d: Doc, patch: impl Fn(&[u8]) -> Vec<u8>, assert_report: impl Fn(&Report)) {
    for reverse in [false, true] {
        let mut d = Doc { entries: d.entries.clone() };
        if reverse {
            d.entries.reverse();
        }
        assert_report(&validate_bytes(&patch(&d.build())));
    }
}
fn unchanged(bytes: &[u8]) -> Vec<u8> {
    bytes.to_vec()
}
fn note_names(report: &Report, name: &str) {
    assert!(report.coverage_notes.iter().any(|n| n.contains(name)), "{report:?}");
}
#[test]
fn n1_p1_corrupt_footnotes_do_not_launder_vacuous_endnotes() {
    orders(
        notes(),
        |b| corrupt_entry_data(b, "word/footnotes.xml"),
        |r| {
            exact(r, Verdict::Incomplete, &[], &["ID-COMMENT-REF", "REL-REF-MISSING"], &["REF-NOTE"]);
            note_names(r, "word/footnotes.xml");
        },
    );
}
#[test]
fn n1_p2_unsupported_comments_do_not_record_the_informational_pass() {
    let mut d = Doc::minimal();
    d.add_with_override("word/comments.xml", CT_COMMENTS, format!(r#"<w:comments {NS}/>"#));
    d.set_doc_rels(&edge("c", "comments", "comments.xml"));
    orders(
        d,
        |b| claim_entry_method(b, "word/comments.xml", 12),
        |r| {
            exact(r, Verdict::Incomplete, &[], &["REF-NOTE", "REL-REF-MISSING"], &["ID-COMMENT-REF"]);
            note_names(r, "word/comments.xml");
        },
    );
}
#[test]
fn n1_p3_corrupt_owner_relationships_skip_all_three_joins() {
    let mut d = Doc::minimal();
    d.set_document(RID);
    orders(
        d,
        |b| corrupt_entry_data(b, "word/_rels/document.xml.rels"),
        |r| {
            exact(r, Verdict::Incomplete, &[], &[], &["REF-NOTE", "ID-COMMENT-REF", "REL-REF-MISSING"]);
            note_names(r, "word/_rels/document.xml.rels");
        },
    );
}
#[test]
fn n1_p4_mid_scan_stop_poisons_every_owned_reference_category() {
    let mut d = notes();
    d.set_document(&NOTE.replace("\"1\"", "\"5\""));
    d.set("word/footnotes.xml", format!(r#"<w:footnotes {NS}>{}</w:footnotes>"#, depth_bomb(r#"<w:footnote w:id="5"><w:p/></w:footnote>"#)));
    orders(d, unchanged, |r| {
        // Amendment 4: unlike a never-loaded source, this scanned prefix invalidates comments too.
        exact(r, Verdict::Incomplete, &[], &["REL-REF-MISSING"], &["REF-NOTE", "ID-COMMENT-REF"]);
        note_names(r, "word/footnotes.xml");
    });
}
#[test]
fn n1_p5_unexamined_suffix_cannot_supply_note_or_comment_discovery() {
    for reverse in [false, true] {
        let mut d = notes();
        let footnotes = d.get("word/footnotes.xml").to_vec();
        d.remove("word/footnotes.xml");
        d.set("[Content_Types].xml", content_types_xml(&[("bin", "application/octet-stream")], &[("/word/footnotes.xml", CT_FOOTNOTES)]));
        // Keep the required entry-cap boundary in both permutations: readable discovery prefix,
        // reversed filler order, and the footnotes source beyond the examined prefix.
        for n in 0..20_000 {
            let i = if reverse { 19_999 - n } else { n };
            d.set(&format!("filler/{i}.bin"), Vec::new());
        }
        d.set("word/footnotes.xml", footnotes);
        let r = validate_bytes(&d.build());
        exact(&r, Verdict::Incomplete, &[], &["REL-REF-MISSING"], &["REF-NOTE", "ID-COMMENT-REF"]);
        assert!(r.coverage_notes.iter().any(|n| n.contains("entry") && n.contains("20000")), "{r:?}");
    }
}
#[test]
fn n1_p6_never_loaded_encrypted_footnotes_do_not_poison_comments() {
    orders(
        notes(),
        |bytes| {
            let mut out = bytes.to_vec();
            for (sig, flags, len, name) in [(b"PK\x03\x04", 6, 26, 30), (b"PK\x01\x02", 8, 28, 46)] {
                for i in 0..out.len().saturating_sub(name) {
                    if out.get(i..i + 4) == Some(sig) {
                        let n = u16::from_le_bytes(out[i + len..i + len + 2].try_into().unwrap()) as usize;
                        if out.get(i + name..i + name + n) == Some(b"word/footnotes.xml") {
                            out[i + flags] |= 1;
                        }
                    }
                }
            }
            out
        },
        |r| {
            exact(r, Verdict::Incomplete, &[], &["ID-COMMENT-REF", "REL-REF-MISSING"], &["REF-NOTE"]);
            note_names(r, "word/footnotes.xml");
        },
    );
}
#[test]
fn n1_p7_capped_relationships_skip_all_three_joins() {
    let mut d = Doc::minimal();
    d.set_document(RID);
    let mut rels = String::new();
    for i in 0..20_001 {
        let id = if i == 20_000 { "last".into() } else { format!("h{i}") };
        rels.push_str(&format!(r#"<Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.invalid/{i}" TargetMode="External"/>"#));
    }
    d.set_doc_rels(&rels);
    orders(d, unchanged, |r| {
        exact(r, Verdict::Incomplete, &[], &[], &["REF-NOTE", "ID-COMMENT-REF", "REL-REF-MISSING"]);
        note_names(r, "word/_rels/document.xml.rels");
    });
}
fn glossary(d: &mut Doc, missing: bool) {
    let old = String::from_utf8(d.get("word/_rels/document.xml.rels").to_vec()).unwrap();
    d.set(
        "word/_rels/document.xml.rels",
        old.replace("</Relationships>", &(edge("g", "glossaryDocument", "glossary/document.xml") + "</Relationships>")),
    );
    d.add_with_override("word/glossary/document.xml", CT_GLOSSARY, format!(r#"<w:glossaryDocument {NS}><w:docParts><w:docPart><w:docPartPr><w:name w:val="g"/></w:docPartPr><w:docPartBody>{NOTE}</w:docPartBody></w:docPart></w:docParts></w:glossaryDocument>"#));
    d.set("word/glossary/_rels/document.xml.rels", rels_xml(&edge("fn", "footnotes", "footnotes.xml")));
    let defs = if missing { String::new() } else { r#"<w:footnote w:id="1"><w:p/></w:footnote>"#.to_string() };
    d.add_with_override("word/glossary/footnotes.xml", CT_FOOTNOTES, format!(r#"<w:footnotes {NS}>{defs}</w:footnotes>"#));
}
#[test]
fn n1_p8_active_glossary_join_survives_main_skip() {
    for missing in [true, false] {
        let mut d = notes();
        glossary(&mut d, missing);
        orders(
            d,
            |b| corrupt_entry_data(b, "word/footnotes.xml"),
            |r| {
                exact(
                    r,
                    if missing { Verdict::Fail } else { Verdict::Incomplete },
                    if missing { &[("REF-NOTE", "word/glossary/document.xml")] } else { &[] },
                    &["REF-NOTE", "ID-COMMENT-REF", "REL-REF-MISSING"],
                    &[],
                );
                note_names(r, "word/footnotes.xml");
            },
        );
    }
}
#[test]
fn n1_p9_vacuous_main_does_not_launder_skipped_glossary() {
    let mut d = Doc::minimal();
    glossary(&mut d, false);
    orders(
        d,
        |b| corrupt_entry_data(b, "word/glossary/footnotes.xml"),
        |r| {
            exact(r, Verdict::Incomplete, &[], &["ID-COMMENT-REF", "REL-REF-MISSING"], &["REF-NOTE"]);
            note_names(r, "word/glossary/footnotes.xml");
        },
    );
}
#[test]
fn n1_p10_minimal_vacuous_scopes_record_each_join_once() {
    orders(Doc::minimal(), unchanged, |r| {
        // validate.rs:117 separately pins the exact ALL_CHECKS order and membership.
        exact(r, Verdict::Pass, &[], &["REF-NOTE", "ID-COMMENT-REF", "REL-REF-MISSING"], &[]);
    });
}
#[test]
fn n1_p11_true_note_absence_still_fails() {
    let mut d = Doc::minimal();
    d.set_document(NOTE);
    orders(d, unchanged, |r| exact(r, Verdict::Fail, &[("REF-NOTE", "word/document.xml")], &["REF-NOTE"], &[]));
}
#[test]
fn n1_p12_true_relationship_absence_still_fails() {
    let mut d = Doc::minimal();
    d.set_document(RID);
    d.remove("word/_rels/document.xml.rels");
    orders(d, unchanged, |r| exact(r, Verdict::Fail, &[("REL-REF-MISSING", "word/document.xml")], &["REL-REF-MISSING"], &[]));
}
#[test]
fn n1_p13_true_comment_absence_still_fails() {
    let mut d = Doc::minimal();
    d.set_document(r#"<w:p><w:r><w:commentReference w:id="1"/></w:r></w:p>"#);
    orders(d, unchanged, |r| exact(r, Verdict::Fail, &[("ID-COMMENT-REF", "word/document.xml")], &["ID-COMMENT-REF"], &[]));
}
#[test]
fn n1_p14_incomplete_reference_prefix_cannot_complete_a_join() {
    for tail in [NOTE, RID] {
        let mut d = Doc::minimal();
        d.set_document(&depth_bomb(tail));
        orders(d, unchanged, |r| {
            exact(r, Verdict::Incomplete, &[], &[], &["REF-NOTE", "ID-COMMENT-REF", "REL-REF-MISSING"]);
            note_names(r, "word/document.xml");
        });
    }
}
#[test]
fn n1_p15_unparsed_directory_does_not_claim_duplicate_check() {
    orders(
        Doc::minimal(),
        |b| {
            let mut bytes = vec![b'X'; 64];
            bytes.extend_from_slice(b);
            bytes
        },
        |r| {
            // Amendment 12 explicitly includes this package check; it is not a residual.
            exact(r, Verdict::Incomplete, &[], &[], &["PKG-DUP-ENTRY"]);
            note_names(r, "central directory could not be parsed");
        },
    );
}
#[test]
fn n1_p16_style_and_font_recording_remains_a_named_residual() {
    let mut d = Doc::minimal();
    d.add_with_override("word/styles.xml", CT_STYLES, format!(r#"<w:styles {NS}><w:style"#));
    d.set_doc_rels(&edge("styles", "styles", "styles.xml"));
    orders(d, unchanged, |r| {
        exact(
            r,
            Verdict::Fail,
            &[("XML-MALFORMED", "word/styles.xml")],
            &["STYLE-DUP-ID", "STYLE-DUP-NAME", "STYLE-RESERVED-NAME", "STYLE-RESERVED-ID", "STYLE-COUNT", "STYLE-ID-LENGTH", "FONT-DUP-NAME"],
            &[],
        );
        note_names(r, "style vocabulary was not checked");
    });
}
