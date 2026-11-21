//! Competing official targets leave owner joins unknown in either relationship order.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[allow(dead_code)]
mod common;

use common::*;
use wordcraft_validate::{Report, Verdict, validate_bytes};

fn rel(id: &str, kind: &str, target: &str) -> String {
    let ns = match kind {
        "commentsExtended" => "http://schemas.microsoft.com/office/2011/relationships",
        "commentsIds" => "http://schemas.microsoft.com/office/2016/09/relationships",
        _ => "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
    };
    format!(r#"<Relationship Id="{id}" Type="{ns}/{kind}" Target="{target}"/>"#)
}

fn pair(kind: &str, reverse: bool, same: bool) -> String {
    let a = rel("a", kind, "a.xml");
    let b = rel("b", kind, if same { "./a.xml" } else { "b.xml" });
    if reverse { b + &a } else { a + &b }
}

fn findings(report: &Report, rules: &[&str]) -> Vec<String> {
    report.findings.iter().filter(|f| rules.contains(&f.rule)).map(|f| format!("{}: {}", f.rule, f.message)).collect()
}

fn ambiguous(report: &Report, kind: &str) {
    assert!(report.coverage_notes.iter().any(|n| n.contains("ambiguous") && n.contains(kind)), "{report:?}");
}

fn unknown(report: &Report, kind: &str, rules: &[&str]) {
    ambiguous(report, kind);
    assert!(findings(report, rules).is_empty(), "{report:?}");
    assert_eq!(report.verdict, Verdict::Incomplete, "{report:?}");
}

fn note_case(kind: &str, competing: &str, reverse: bool, same: bool) -> Doc {
    let mut d = Doc::minimal();
    let ct = if kind == "footnotes" { CT_FOOTNOTES } else { CT_ENDNOTES };
    let singular = if kind == "footnotes" { "footnote" } else { "endnote" };
    let settings = format!(r#"<w:settings {NS}><w:{singular}Pr><w:{singular} w:id="7"/></w:{singular}Pr></w:settings>"#);
    let notes = format!(r#"<w:{kind} {NS}><w:{singular} w:type="separator" w:id="-1"><w:p/></w:{singular}></w:{kind}>"#);
    if competing == "settings" {
        d.add_with_override("word/a.xml", CT_SETTINGS, settings);
        d.add_with_override("word/b.xml", CT_SETTINGS, format!(r#"<w:settings {NS}/>"#));
        d.add_with_override("word/notes.xml", ct, notes);
        d.set_doc_rels(&(pair(competing, reverse, same) + &rel("notes", kind, "notes.xml")));
    } else {
        d.add_with_override("word/a.xml", ct, notes);
        d.add_with_override("word/b.xml", ct, format!(r#"<w:{kind} {NS}/>"#));
        d.add_with_override("word/settings.xml", CT_SETTINGS, settings);
        d.set_doc_rels(&(pair(competing, reverse, same) + &rel("settings", "settings", "settings.xml")));
    }
    d
}

#[test]
fn note_joins_suppress_both_directions_for_each_competing_role_in_both_orders() {
    for kind in ["footnotes", "endnotes"] {
        for competing in ["settings", kind] {
            for reverse in [false, true] {
                let report = validate_bytes(&note_case(kind, competing, reverse, false).build());
                unknown(&report, competing, &["NOTE-PR-LISTING", "NOTE-PR-DANGLING"]);
            }
        }
    }
}

#[test]
fn repeated_target_keeps_note_findings_in_both_orders() {
    for kind in ["footnotes", "endnotes"] {
        for competing in ["settings", kind] {
            for reverse in [false, true] {
                let report = validate_bytes(&note_case(kind, competing, reverse, true).build());
                assert!(!report.coverage_notes.iter().any(|n| n.contains("ambiguous")), "{report:?}");
                assert_eq!(findings(&report, &["NOTE-PR-LISTING", "NOTE-PR-DANGLING"]).len(), 2, "{report:?}");
                assert_eq!(report.verdict, Verdict::Fail);
            }
        }
    }
}

#[test]
fn malformed_competing_note_sources_keep_independent_xml_errors_in_both_orders() {
    for kind in ["footnotes", "endnotes"] {
        for competing in ["settings", kind] {
            for reverse in [false, true] {
                let mut d = note_case(kind, competing, reverse, false);
                d.set("word/b.xml", format!(r#"<w:{competing} {NS}>"#));
                let report = validate_bytes(&d.build());
                ambiguous(&report, competing);
                assert!(findings(&report, &["NOTE-PR-LISTING", "NOTE-PR-DANGLING"]).is_empty(), "{report:?}");
                assert!(report.findings.iter().any(|f| f.rule == "XML-MALFORMED" && f.part.as_deref() == Some("word/b.xml")), "{report:?}");
                assert_eq!(report.verdict, Verdict::Fail);
            }
        }
    }
}

#[test]
fn owned_ids_do_not_choose_competing_definitions_in_either_order() {
    for (kind, ct, element, rule) in [
        ("footnotes", CT_FOOTNOTES, "footnote", "REF-NOTE"),
        ("endnotes", CT_ENDNOTES, "endnote", "REF-NOTE"),
        ("comments", CT_COMMENTS, "comment", "ID-COMMENT-REF"),
    ] {
        for reverse in [false, true] {
            for same in [false, true] {
                let mut d = Doc::minimal();
                d.set_document(&format!(r#"<w:p><w:r><w:{element}Reference w:id="9"/></w:r></w:p>"#));
                d.add_with_override("word/a.xml", ct, format!(r#"<w:{kind} {NS}><w:{element} w:id="1"><w:p/></w:{element}></w:{kind}>"#));
                d.add_with_override("word/b.xml", ct, format!(r#"<w:{kind} {NS}><w:{element} w:id="9"><w:p/></w:{element}></w:{kind}>"#));
                d.set_doc_rels(&pair(kind, reverse, same));
                let report = validate_bytes(&d.build());
                if same {
                    assert_eq!(findings(&report, &[rule]).len(), 1, "{report:?}");
                } else {
                    unknown(&report, kind, &[rule]);
                }
            }
        }
    }
}

fn comments(para: &str) -> String {
    format!(
        r#"<w:comments {NS} xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:comment w:id="1"><w:p w14:paraId="{para}"/></w:comment></w:comments>"#
    )
}

fn ext(para: &str, parent: &str) -> String {
    format!(
        r#"<w15:commentsEx xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml"><w15:commentEx w15:paraId="{para}" {parent}/></w15:commentsEx>"#
    )
}

fn cid(para: &str) -> String {
    format!(
        r#"<w16cid:commentsIds xmlns:w16cid="http://schemas.microsoft.com/office/word/2016/wordml/cid"><w16cid:commentId w16cid:paraId="{para}" w16cid:durableId="00000001"/></w16cid:commentsIds>"#
    )
}

#[test]
fn comment_extension_joins_handle_each_competing_source_in_both_orders() {
    for kind in ["comments", "commentsExtended", "commentsIds"] {
        for reverse in [false, true] {
            for same in [false, true] {
                let mut d = Doc::minimal();
                let (ct, a, b) = match kind {
                    "comments" => (CT_COMMENTS, comments("00000001"), comments("00000002")),
                    "commentsExtended" => (CT_COMMENTS_EXT, ext("00000002", ""), ext("00000001", "")),
                    _ => (CT_COMMENTS_IDS, cid("00000003"), cid("00000002")),
                };
                d.add_with_override("word/a.xml", ct, a);
                d.add_with_override("word/b.xml", ct, b);
                let mut rels = pair(kind, reverse, same);
                if kind != "comments" {
                    d.add_with_override("word/comments.xml", CT_COMMENTS, comments("00000002"));
                    rels += &rel("comments", "comments", "comments.xml");
                }
                if kind != "commentsExtended" {
                    d.add_with_override("word/ext.xml", CT_COMMENTS_EXT, ext("00000002", ""));
                    rels += &rel("ext", "commentsExtended", "ext.xml");
                }
                if kind == "commentsExtended" {
                    d.add_with_override("word/cid.xml", CT_COMMENTS_IDS, cid("00000003"));
                    rels += &rel("cid", "commentsIds", "cid.xml");
                }
                d.set_doc_rels(&rels);
                let report = validate_bytes(&d.build());
                if same {
                    assert!(!findings(&report, &["COMMENTEX-LINK"]).is_empty(), "{report:?}");
                } else {
                    unknown(&report, kind, &["COMMENTEX-LINK"]);
                }
            }
        }
    }
}

#[test]
fn comment_parent_findings_are_independent_of_ambiguous_comments_in_both_orders() {
    for reverse in [false, true] {
        let mut d = Doc::minimal();
        d.add_with_override("word/a.xml", CT_COMMENTS, comments("00000001"));
        d.add_with_override("word/b.xml", CT_COMMENTS, comments("00000002"));
        d.add_with_override("word/ext.xml", CT_COMMENTS_EXT, ext("00000003", r#"w15:paraIdParent="00000004""#));
        d.set_doc_rels(&(pair("comments", reverse, false) + &rel("ext", "commentsExtended", "ext.xml")));
        let report = validate_bytes(&d.build());
        ambiguous(&report, "comments");
        let links = findings(&report, &["COMMENTEX-LINK"]);
        assert_eq!(links.len(), 1, "{report:?}");
        assert!(links[0].contains("paraIdParent"));
    }
}

#[test]
fn styles_fonts_numbering_and_schemas_do_not_use_competing_sources_in_either_order() {
    for kind in ["styles", "fontTable", "numbering", "settings"] {
        for reverse in [false, true] {
            for same in [false, true] {
                let mut d = Doc::minimal();
                let (ct, a, b, rules) = match kind {
                    "styles" => {
                        d.set_document(r#"<w:p><w:pPr><w:pStyle w:val="Missing"/></w:pPr></w:p>"#);
                        (
                            CT_STYLES,
                            format!(r#"<w:styles {NS}><w:style w:styleId="A"/><w:style w:styleId="A"/></w:styles>"#),
                            format!(r#"<w:styles {NS}/>"#),
                            vec!["REF-STYLE", "STYLE-DUP-ID"],
                        )
                    }
                    "fontTable" => (
                        CT_FONT_TABLE,
                        format!(r#"<w:fonts {NS}><w:font w:name="A"/><w:font w:name="A"/></w:fonts>"#),
                        format!(r#"<w:fonts {NS}/>"#),
                        vec!["FONT-DUP-NAME"],
                    ),
                    "numbering" => {
                        d.set_document(r#"<w:p><w:pPr><w:numPr><w:numId w:val="9"/></w:numPr></w:pPr></w:p>"#);
                        (
                            CT_NUMBERING,
                            format!(r#"<w:numbering {NS}><w:num w:numId="1"><w:abstractNumId w:val="7"/></w:num></w:numbering>"#),
                            format!(r#"<w:numbering {NS}/>"#),
                            vec!["NUM-DANGLING"],
                        )
                    }
                    _ => {
                        d.add_with_override("word/item.xml", CT_CUSTOMXML_PROPS, r#"<ds:datastoreItem xmlns:ds="http://schemas.openxmlformats.org/officeDocument/2006/customXml"><ds:schemaRefs><ds:schemaRef ds:uri="urn:missing"/></ds:schemaRefs></ds:datastoreItem>"#);
                        (
                            CT_SETTINGS,
                            format!(r#"<w:settings {NS}/>"#),
                            format!(r#"<w:settings {NS}><w:attachedSchema w:val="urn:missing"/></w:settings>"#),
                            vec!["CUSTOMXML-SCHEMA"],
                        )
                    }
                };
                d.add_with_override("word/a.xml", ct, a);
                d.add_with_override("word/b.xml", ct, b);
                d.set_doc_rels(&pair(kind, reverse, same));
                let report = validate_bytes(&d.build());
                if same {
                    assert!(!findings(&report, &rules).is_empty(), "{report:?}");
                } else {
                    unknown(&report, kind, &rules);
                    if kind == "styles" {
                        assert!(
                            !report
                                .coverage_notes
                                .iter()
                                .any(|n| n.contains("no styles part is reachable") || n.contains("present but does not parse")),
                            "{report:?}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn ambiguous_styles_do_not_supply_numbering_references_in_both_orders() {
    for reverse in [false, true] {
        let mut d = Doc::minimal();
        d.add_with_override(
            "word/a.xml",
            CT_STYLES,
            format!(r#"<w:styles {NS}><w:style w:styleId="A"><w:pPr><w:numPr><w:numId w:val="9"/></w:numPr></w:pPr></w:style></w:styles>"#),
        );
        d.add_with_override("word/b.xml", CT_STYLES, format!(r#"<w:styles {NS}/>"#));
        d.set_doc_rels(&pair("styles", reverse, false));
        unknown(&validate_bytes(&d.build()), "styles", &["NUM-DANGLING"]);
    }
}

#[test]
fn disjoint_owners_keep_clean_note_joins_in_both_directions_and_orders() {
    for ambiguous_main in [false, true] {
        for reverse in [false, true] {
            let mut d = note_case("footnotes", "settings", reverse, false);
            d.add_with_override("word/glossary.xml", CT_GLOSSARY, format!(r#"<w:glossaryDocument {NS}><w:docParts/></w:glossaryDocument>"#));
            d.add_with_override(
                "word/clean-notes.xml",
                CT_FOOTNOTES,
                format!(r#"<w:footnotes {NS}><w:footnote w:type="separator" w:id="-2"><w:p/></w:footnote></w:footnotes>"#),
            );
            let ambiguous_rels = pair("settings", reverse, false) + &rel("notes", "footnotes", "notes.xml");
            let clean_rels = rel("notes", "footnotes", "clean-notes.xml");
            let (main_rels, glossary_rels) = if ambiguous_main { (ambiguous_rels, clean_rels) } else { (clean_rels, ambiguous_rels) };
            d.set_doc_rels(&(main_rels + &rel("glossary", "glossaryDocument", "glossary.xml")));
            d.set("word/_rels/glossary.xml.rels", rels_xml(&glossary_rels));
            let report = validate_bytes(&d.build());
            ambiguous(&report, "settings");
            let notes = findings(&report, &["NOTE-PR-LISTING", "NOTE-PR-DANGLING"]);
            assert_eq!(notes.len(), 1, "{report:?}");
            assert!(notes[0].contains("clean-notes.xml"), "{report:?}");
            assert_eq!(report.verdict, Verdict::Fail);
        }
    }
}

#[test]
fn competing_main_and_glossary_roots_gate_document_checks_in_both_orders() {
    for kind in ["officeDocument", "glossaryDocument"] {
        for reverse in [false, true] {
            for same in [false, true] {
                let mut d = Doc::minimal();
                let is_main = kind == "officeDocument";
                if is_main {
                    d.remove("word/document.xml");
                }
                let (ct, root) = if is_main { (CT_MAIN, "document") } else { (CT_GLOSSARY, "glossaryDocument") };
                for part in ["a.xml", "b.xml"] {
                    d.add_with_override(
                        &format!("word/{part}"),
                        if is_main && part == "b.xml" { "application/xml" } else { ct },
                        format!(r#"<w:{root} {NS}><w:p><w:r><w:footnoteReference w:id="9"/></w:r></w:p></w:{root}>"#),
                    );
                }
                if is_main {
                    let a = rel("a", kind, "word/a.xml");
                    let b = rel("b", kind, if same { "word/./a.xml" } else { "word/b.xml" });
                    d.set("_rels/.rels", rels_xml(&(if reverse { b + &a } else { a + &b })));
                } else {
                    d.set_doc_rels(&pair(kind, reverse, same));
                }
                let report = validate_bytes(&d.build());
                if same {
                    assert!(!report.coverage_notes.iter().any(|n| n.contains("ambiguous")), "{report:?}");
                    assert!(!findings(&report, &["REF-NOTE"]).is_empty(), "{report:?}");
                } else {
                    unknown(&report, kind, &["REF-NOTE", "PKG-NO-MAIN"]);
                    assert!(!report.checks_run.contains(&"REF-NOTE"), "{report:?}");
                }
            }
        }
    }
}

#[test]
fn dangling_competitors_still_make_selection_unknown_in_both_orders() {
    for reverse in [false, true] {
        let mut d = note_case("footnotes", "settings", reverse, false);
        d.remove("word/b.xml");
        let report = validate_bytes(&d.build());
        ambiguous(&report, "settings");
        assert!(findings(&report, &["NOTE-PR-LISTING", "NOTE-PR-DANGLING"]).is_empty(), "{report:?}");
        assert!(!findings(&report, &["REL-DANGLING"]).is_empty(), "{report:?}");
        assert_eq!(report.verdict, Verdict::Fail);
    }
}

#[test]
fn ambiguous_comments_do_not_establish_unreferenced_candidates_in_both_orders() {
    for reverse in [false, true] {
        let mut d = Doc::minimal();
        d.add_with_override("word/a.xml", CT_COMMENTS, comments("00000001"));
        d.add_with_override("word/b.xml", CT_COMMENTS, comments("00000002"));
        d.set_doc_rels(&pair("comments", reverse, false));
        let report = validate_bytes(&d.build());
        unknown(&report, "comments", &["ID-COMMENT-REF"]);
        assert!(!report.coverage_notes.iter().any(|n| n.contains("never referenced")), "{report:?}");
    }
}

#[test]
fn one_note_kind_ambiguity_keeps_the_other_kinds_findings_in_both_orders() {
    for reverse in [false, true] {
        let mut d = note_case("footnotes", "footnotes", reverse, false);
        d.add_with_override(
            "word/endnotes.xml",
            CT_ENDNOTES,
            format!(r#"<w:endnotes {NS}><w:endnote w:type="separator" w:id="-2"><w:p/></w:endnote></w:endnotes>"#),
        );
        d.set_doc_rels(
            &(pair("footnotes", reverse, false) + &rel("settings", "settings", "settings.xml") + &rel("endnotes", "endnotes", "endnotes.xml")),
        );
        let report = validate_bytes(&d.build());
        ambiguous(&report, "footnotes");
        let notes = findings(&report, &["NOTE-PR-LISTING", "NOTE-PR-DANGLING"]);
        assert_eq!(notes.len(), 1, "{report:?}");
        assert!(notes[0].contains("endnotes.xml"), "{report:?}");
        assert_eq!(report.verdict, Verdict::Fail);
    }
}

#[test]
fn ambiguous_roots_keep_independent_xml_and_existing_cardinality_findings() {
    for kind in ["officeDocument", "glossaryDocument"] {
        for reverse in [false, true] {
            let mut d = Doc::minimal();
            let (ct, root) = if kind == "officeDocument" { (CT_MAIN, "document") } else { (CT_GLOSSARY, "glossaryDocument") };
            d.add_with_override("word/a.xml", ct, format!(r#"<w:{root} {NS}>"#));
            d.add_with_override("word/b.xml", ct, format!(r#"<w:{root} {NS}/>"#));
            if kind == "officeDocument" {
                let a = rel("a", kind, "word/a.xml");
                let b = rel("b", kind, "word/b.xml");
                d.set("_rels/.rels", rels_xml(&(if reverse { b + &a } else { a + &b })));
            } else {
                d.set_doc_rels(&pair(kind, reverse, false));
            }
            let report = validate_bytes(&d.build());
            ambiguous(&report, kind);
            assert!(report.findings.iter().any(|f| f.rule == "XML-MALFORMED" && f.part.as_deref() == Some("word/a.xml")), "{report:?}");
            assert_eq!(report.verdict, Verdict::Fail);
            if kind == "officeDocument" {
                assert!(!findings(&report, &["CT-DUP-MAIN"]).is_empty(), "{report:?}");
            }
        }
    }
}

#[test]
fn malformed_settings_without_notes_fail_independently_of_suppressed_note_joins() {
    let mut d = Doc::minimal();
    d.add_with_override("word/settings.xml", CT_SETTINGS, format!(r#"<w:settings {NS}><w:footnotePr>"#));
    d.set_doc_rels(&rel("settings", "settings", "settings.xml"));
    let report = validate_bytes(&d.build());
    assert_eq!(report.verdict, Verdict::Fail, "{report:?}");
    assert!(!findings(&report, &["XML-MALFORMED"]).is_empty(), "{report:?}");
    assert!(findings(&report, &["NOTE-PR-LISTING", "NOTE-PR-DANGLING"]).is_empty(), "{report:?}");
    assert!(report.coverage_notes.iter().any(|n| n.contains("footnotePr")), "{report:?}");
}
