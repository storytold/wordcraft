//! Selected owner roles reject orphan sources despite unrelated discovery loss.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[allow(dead_code)]
mod common;

use common::{CT_COMMENTS, CT_COMMENTS_EXT, CT_ENDNOTES, CT_FOOTNOTES, CT_HEADER, CT_SETTINGS, Doc, NS, rels_xml};
use wordcraft_validate::{Report, Severity, Verdict, validate_bytes};

fn rel(kind: &str, target: &str) -> String {
    format!(r#"<Relationship Id="{target}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}" Target="{target}"/>"#)
}

fn fixture(mode: &str) -> Doc {
    let mut d = Doc::minimal();
    d.set("word/unrelated.xml", "<unrelated/>");
    d.set(
        "word/_rels/unrelated.xml.rels",
        match mode {
            "encoding" => br#"<?xml version="1.0" encoding="unsupported-test-encoding"?><Relationships/>"#.to_vec(),
            "malformed" => b"<Relationships><broken></Relationships>".to_vec(),
            _ => rels_xml(""),
        },
    );
    d
}

/// Select the source directly; optionally retain a second path through an established header.
fn establish(d: &mut Doc, main: &str, source: &str, descendant: bool) {
    if descendant {
        d.add_with_override("word/branch.xml", CT_HEADER, format!(r#"<w:hdr {NS}><w:p/></w:hdr>"#));
        d.set_doc_rels(&(main.to_string() + source + &rel("header", "branch.xml")));
        d.set("word/_rels/branch.xml.rels", rels_xml(source));
    } else {
        d.set_doc_rels(&(main.to_string() + source));
    }
}

fn findings(report: &Report, rule: &str) -> usize {
    report.findings.iter().filter(|f| f.rule == rule).count()
}

/// Neither an absent nor an empty selected settings source can consume an orphan listing.
#[test]
fn round9_settings_role_boundary() {
    for (note, ct) in [("footnote", CT_FOOTNOTES), ("endnote", CT_ENDNOTES)] {
        for descendant in [false, true] {
            for mode in ["encoding", "malformed", "complete"] {
                for filled in [true, false] {
                    let mut d = fixture(mode);
                    d.add_with_override(
                        "word/notes.xml",
                        ct,
                        format!(r#"<w:{note}s {NS}><w:{note} w:id="1" w:type="separator"><w:p/></w:{note}></w:{note}s>"#),
                    );
                    d.add_with_override(
                        "word/fallback.xml",
                        CT_SETTINGS,
                        format!(r#"<w:settings {NS}><w:{note}Pr><w:{note} w:id="1"/></w:{note}Pr></w:settings>"#),
                    );
                    let source = if filled {
                        d.add_with_override("word/selected.xml", CT_SETTINGS, format!(r#"<w:settings {NS}/>"#));
                        rel("settings", "selected.xml")
                    } else {
                        String::new()
                    };
                    establish(&mut d, &rel(&format!("{note}s"), "notes.xml"), &source, descendant);
                    let report = validate_bytes(&d.build());
                    eprintln!("{note} settings filled={filled} descendant={descendant} {mode}: {report:?}");
                    let missing = true;
                    assert_eq!(findings(&report, "NOTE-PR-LISTING"), usize::from(missing), "{report:?}");
                    if missing {
                        assert_eq!(report.verdict, Verdict::Fail, "{report:?}");
                        assert!(
                            report
                                .findings
                                .iter()
                                .any(|f| f.rule == "NOTE-PR-LISTING" && f.severity == Severity::Error && f.part.as_deref() == Some("word/notes.xml")),
                            "{report:?}"
                        );
                    } else {
                        assert_eq!(report.verdict, if mode == "complete" { Verdict::Pass } else { Verdict::Incomplete }, "{report:?}");
                    }
                }
            }
        }
    }
}

/// A filled footnotes role cannot fill endnotes, or the reverse, in the same owner context.
#[test]
fn round9_notes_role_boundary_is_per_kind() {
    for (note, ct, other, other_ct) in [("footnote", CT_FOOTNOTES, "endnote", CT_ENDNOTES), ("endnote", CT_ENDNOTES, "footnote", CT_FOOTNOTES)] {
        for descendant in [false, true] {
            for mode in ["encoding", "malformed", "complete"] {
                for filled in [true, false] {
                    let mut d = fixture(mode);
                    d.add_with_override("word/settings.xml", CT_SETTINGS, format!(r#"<w:settings {NS}><w:footnotePr><w:footnote w:id="1"/></w:footnotePr><w:endnotePr><w:endnote w:id="1"/></w:endnotePr></w:settings>"#));
                    for (name, kind, content_type, id) in [("fallback.xml", note, ct, "1"), ("other.xml", other, other_ct, "1")] {
                        d.add_with_override(
                            &format!("word/{name}"),
                            content_type,
                            format!(r#"<w:{kind}s {NS}><w:{kind} w:id="{id}"><w:p/></w:{kind}></w:{kind}s>"#),
                        );
                    }
                    let source = if filled {
                        d.add_with_override("word/selected.xml", ct, format!(r#"<w:{note}s {NS}><w:{note} w:id="2"><w:p/></w:{note}></w:{note}s>"#));
                        rel(&format!("{note}s"), "selected.xml")
                    } else {
                        String::new()
                    };
                    establish(&mut d, &(rel("settings", "settings.xml") + &rel(&format!("{other}s"), "other.xml")), &source, descendant);
                    let report = validate_bytes(&d.build());
                    eprintln!("{note} notes filled={filled} descendant={descendant} {mode}: {report:?}");
                    let missing = true;
                    assert_eq!(findings(&report, "NOTE-PR-DANGLING"), usize::from(missing), "{report:?}");
                    if missing {
                        assert!(
                            report
                                .findings
                                .iter()
                                .any(|f| f.rule == "NOTE-PR-DANGLING" && f.message.contains(&format!("a {note}Pr lists {note} id 1,"))),
                            "{report:?}"
                        );
                    }
                    assert_eq!(report.verdict, if mode == "complete" { Verdict::Pass } else { Verdict::Incomplete }, "{report:?}");
                }
            }
        }
    }
}

/// An orphan paragraph cannot satisfy the selected comments role, including an empty target.
#[test]
fn round9_comments_role_boundary() {
    for descendant in [false, true] {
        for mode in ["encoding", "malformed", "complete"] {
            for filled in [true, false] {
                let mut d = fixture(mode);
                d.add_with_override("word/ex.xml", CT_COMMENTS_EXT, r#"<w15:commentsEx xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml"><w15:commentEx w15:paraId="00000003"/></w15:commentsEx>"#);
                d.add_with_override("word/fallback.xml", CT_COMMENTS, format!(r#"<w:comments {NS} xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:comment w:id="1"><w:p w14:paraId="00000003"/></w:comment></w:comments>"#));
                let source = if filled {
                    d.add_with_override("word/selected.xml", CT_COMMENTS, format!(r#"<w:comments {NS}/>"#));
                    rel("comments", "selected.xml")
                } else {
                    String::new()
                };
                establish(&mut d, &rel("commentsExtended", "ex.xml"), &source, descendant);
                let report = validate_bytes(&d.build());
                eprintln!("comments filled={filled} descendant={descendant} {mode}: {report:?}");
                assert_eq!(
                    report.findings.iter().filter(|f| f.rule == "COMMENTEX-LINK" && f.message.contains("no paragraph")).count(),
                    1,
                    "{report:?}"
                );
                assert_eq!(report.verdict, if mode == "complete" { Verdict::Pass } else { Verdict::Incomplete }, "{report:?}");
            }
        }
    }
}
