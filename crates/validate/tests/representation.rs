//! Numeric identity regressions for the representation lane.
#![allow(clippy::unwrap_used, clippy::expect_used)]
#[allow(dead_code)]
mod common;
use common::{CT_COMMENTS, CT_COMMENTS_EXT, CT_COMMENTS_IDS, CT_NUMBERING, CT_STYLES, Doc, NS};
use wordcraft_validate::{Report, validate_bytes};

fn rel(kind: &str, target: &str) -> String {
    format!(r#"<Relationship Id="{target}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}" Target="{target}"/>"#)
}

fn without(report: &Report, rules: &[&str]) {
    assert!(!report.findings.iter().any(|f| rules.contains(&f.rule)), "{report:?}");
}

/// Compare the entire serialized report for each independent ZIP/relationship-order permutation.
fn report_in_all_orders(d: &mut Doc) -> Report {
    let original = d.entries.clone();
    let expected = validate_bytes(&d.build());
    let json = serde_json::to_vec(&expected).unwrap();
    for reverse_zip in [false, true] {
        for reverse_relationships in [false, true] {
            d.entries = original.clone();
            if reverse_relationships {
                for (name, bytes) in &mut d.entries {
                    if name.ends_with(".rels") {
                        let text = String::from_utf8(bytes.clone()).unwrap();
                        if let Some(start) = text.find("<Relationship ") {
                            let (prefix, body) = text.split_at(start);
                            let body = body.strip_suffix("</Relationships>").unwrap();
                            let mut edges: Vec<_> = body.split_inclusive("/>").collect();
                            edges.reverse();
                            *bytes = format!("{prefix}{}</Relationships>", edges.concat()).into_bytes();
                        }
                    }
                }
            }
            if reverse_zip {
                d.entries.reverse();
            }
            assert_eq!(serde_json::to_vec(&validate_bytes(&d.build())).unwrap(), json);
        }
    }
    d.entries = original;
    expected
}

#[test]
fn decimal_identity_joins_bookmarks_moves_numbering_and_style_numbering() {
    for id in ["01", "+0001", " 1 ", "000000000000000000000000000000000000000000000001"] {
        let mut d = Doc::minimal();
        d.set_document(&format!(r#"<w:p><w:pPr><w:numPr><w:numId w:val="{id}"/></w:numPr></w:pPr><w:bookmarkStart w:id="1" w:name="b"/><w:bookmarkEnd w:id="{id}"/><w:moveFromRangeStart w:id="1" w:name="m"/><w:moveFromRangeEnd w:id="{id}"/><w:moveToRangeStart w:id="2" w:name="m"/><w:moveToRangeEnd w:id="02"/></w:p>"#));
        d.add_with_override(
            "word/numbering.xml",
            CT_NUMBERING,
            format!(
                r#"<w:numbering {NS}><w:abstractNum w:abstractNumId="{id}"/><w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num></w:numbering>"#
            ),
        );
        d.add_with_override(
            "word/styles.xml",
            CT_STYLES,
            format!(r#"<w:styles {NS}><w:style w:styleId="s"><w:pPr><w:numPr><w:numId w:val="{id}"/></w:numPr></w:pPr></w:style></w:styles>"#),
        );
        d.set_doc_rels(&(rel("numbering", "numbering.xml") + &rel("styles", "styles.xml")));
        without(&report_in_all_orders(&mut d), &["NUM-DANGLING", "BOOKMARK-UNPAIRED", "STRUCT-MOVE-UNPAIRED"]);
    }
}

#[test]
fn numeric_zero_numbering_sentinel_and_lexical_style_ids_stay_distinct() {
    for zero in ["00", "+0", "-000", " 000 "] {
        let mut d = Doc::minimal();
        d.set_document(&format!(r#"<w:p><w:pPr><w:numPr><w:numId w:val="{zero}"/></w:numPr><w:pStyle w:val="01"/></w:pPr></w:p>"#));
        d.add_with_override(
            "word/styles.xml",
            CT_STYLES,
            format!(r#"<w:styles {NS}><w:style w:styleId="1"><w:pPr><w:numPr><w:numId w:val="{zero}"/></w:numPr></w:pPr></w:style></w:styles>"#),
        );
        d.set_doc_rels(&rel("styles", "styles.xml"));
        let report = report_in_all_orders(&mut d);
        without(&report, &["NUM-DANGLING"]);
        assert!(report.findings.iter().any(|f| f.rule == "REF-STYLE"), "{report:?}");
    }
}

#[test]
fn decimal_duplicates_use_value_keys_for_all_migrated_families() {
    let mut d = Doc::minimal();
    d.set_document(r#"<w:p><w:bookmarkStart w:id="1" w:name="a"/><w:bookmarkStart w:id="01" w:name="b"/><w:bookmarkEnd w:id="1"/><w:ins w:id="2"/><w:del w:id="+02"/><w:moveFromRangeStart w:id="3"/><w:moveToRangeStart w:id="003"/><w:moveFromRangeEnd w:id="3"/><w:moveToRangeEnd w:id="3"/></w:p>"#);
    d.add_with_override("word/numbering.xml", CT_NUMBERING, format!(r#"<w:numbering {NS}><w:abstractNum w:abstractNumId="1"/><w:abstractNum w:abstractNumId="01"/><w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num><w:num w:numId="+01"><w:abstractNumId w:val="01"/></w:num></w:numbering>"#));
    d.set_doc_rels(&rel("numbering", "numbering.xml"));
    let report = report_in_all_orders(&mut d);
    for rule in ["ID-BOOKMARK-DUP", "ID-REVISION-DUP", "STRUCT-MOVE-UNPAIRED", "NUM-DANGLING"] {
        assert!(report.findings.iter().any(|f| f.rule == rule), "missing {rule}: {report:?}");
    }
    assert_eq!(report.findings.iter().filter(|f| f.rule == "NUM-DANGLING").count(), 2);
}

#[test]
fn comment_threading_hex_padding_case_and_whitespace_join() {
    for id in ["1", "0001", " 00000001 "] {
        let mut d = Doc::minimal();
        d.add_with_override("word/comments.xml", CT_COMMENTS, format!(r#"<w:comments {NS} xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:comment w:id="1"><w:p w14:paraId="00000001"/></w:comment><w:comment w:id="2"><w:p w14:paraId="0000000A"/></w:comment></w:comments>"#));
        d.add_with_override("word/ext.xml", CT_COMMENTS_EXT, format!(r#"<w15:commentsEx xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml"><w15:commentEx w15:paraId="{id}"/><w15:commentEx w15:paraId="a" w15:paraIdParent="{id}"/></w15:commentsEx>"#));
        d.add_with_override("word/ids.xml", CT_COMMENTS_IDS, r#"<w16cid:commentsIds xmlns:w16cid="http://schemas.microsoft.com/office/word/2016/wordml/cid"><w16cid:commentId w16cid:paraId="00000001"/><w16cid:commentId w16cid:paraId=" 000A "/></w16cid:commentsIds>"#);
        d.set_doc_rels(&(rel("comments", "comments.xml") + &rel("commentsExtended", "ext.xml") + &rel("commentsIds", "ids.xml")));
        without(&report_in_all_orders(&mut d), &["COMMENTEX-LINK"]);
    }
}

#[test]
fn invalid_numeric_spellings_do_not_coerce_to_valid_keys() {
    for id in ["1.0", "1e0", "++1", "0x1"] {
        let mut d = Doc::minimal();
        d.set_document(&format!(r#"<w:p><w:bookmarkStart w:id="1" w:name="b"/><w:bookmarkEnd w:id="{id}"/></w:p>"#));
        let report = report_in_all_orders(&mut d);
        assert_eq!(report.findings.iter().filter(|f| f.rule == "BOOKMARK-UNPAIRED").count(), 2, "{report:?}");
    }
}

#[test]
fn arbitrary_length_signed_decimal_values_pair_without_machine_integer_conversion() {
    for id in ["12345678901234567890123456789012345678901234567890", "-12345678901234567890123456789012345678901234567890"] {
        let mut d = Doc::minimal();
        let padded = if let Some(digits) = id.strip_prefix('-') { format!("-000{digits}") } else { format!("+000{id}") };
        d.set_document(&format!(r#"<w:p><w:bookmarkStart w:id="{id}" w:name="b"/><w:bookmarkEnd w:id="{padded}"/></w:p>"#));
        without(&report_in_all_orders(&mut d), &["BOOKMARK-UNPAIRED"]);
    }
}

#[test]
fn relationship_ids_remain_lexical() {
    let mut d = Doc::minimal();
    d.set_document(r#"<w:p><w:hyperlink r:id="01"/></w:p>"#);
    d.set_doc_rels(r#"<Relationship Id="1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.invalid/" TargetMode="External"/>"#);
    let report = report_in_all_orders(&mut d);
    assert!(report.findings.iter().any(|f| f.rule == "REL-REF-MISSING"), "{report:?}");
}

#[test]
fn unreached_story_references_never_borrow_main_definitions() {
    use common::{CT_HEADER, CT_SETTINGS};
    let mut d = Doc::minimal();
    d.add_with_override("word/orphan.xml", CT_HEADER, format!(r#"<w:hdr {NS}><w:p><w:pPr><w:pStyle w:val="missing"/><w:numPr><w:numId w:val="9"/></w:numPr></w:pPr><w:r><w:footnoteReference w:id="9"/><w:endnoteReference w:id="9"/><w:commentReference w:id="9"/></w:r></w:p></w:hdr>"#));
    d.add_with_override("word/styles.xml", CT_STYLES, format!(r#"<w:styles {NS}/>"#));
    d.add_with_override("word/settings.xml", CT_SETTINGS, format!(r#"<w:settings {NS}/>"#));
    let sources = rel("styles", "styles.xml") + &rel("settings", "settings.xml");
    d.set_doc_rels(&sources);
    let orphan = report_in_all_orders(&mut d);
    without(&orphan, &["REF-NOTE", "ID-COMMENT-REF", "REF-STYLE", "NUM-DANGLING"]);
    // The same references really reached from main must still be diagnosed.
    d.set_doc_rels(&(sources + &rel("header", "orphan.xml")));
    let reached = report_in_all_orders(&mut d);
    for rule in ["REF-NOTE", "ID-COMMENT-REF", "REF-STYLE", "NUM-DANGLING"] {
        assert!(reached.findings.iter().any(|f| f.rule == rule && f.part.as_deref() == Some("word/orphan.xml")), "{reached:?}");
    }
}

#[test]
fn either_comment_extension_activates_only_its_established_directions() {
    for ids in [false, true] {
        let mut d = Doc::minimal();
        if ids {
            d.add_with_override("word/ids.xml", CT_COMMENTS_IDS, r#"<w16cid:commentsIds xmlns:w16cid="http://schemas.microsoft.com/office/word/2016/wordml/cid"><w16cid:commentId w16cid:paraId="1"/></w16cid:commentsIds>"#);
            d.set_doc_rels(&rel("commentsIds", "ids.xml"));
        } else {
            d.add_with_override("word/ext.xml", CT_COMMENTS_EXT, r#"<w15:commentsEx xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml"><w15:commentEx w15:paraId="1" w15:paraIdParent="2"/></w15:commentsEx>"#);
            d.set_doc_rels(&rel("commentsExtended", "ext.xml"));
        }
        let report = report_in_all_orders(&mut d);
        assert!(report.checks_run.contains(&"COMMENTEX-LINK"), "{report:?}");
        if ids {
            // Amendment 1: established-absent optional counterpart is not a mismatch.
            without(&report, &["COMMENTEX-LINK"]);
        } else {
            assert!(report.findings.iter().any(|f| f.rule == "COMMENTEX-LINK" && f.message.contains("no paragraph")), "{report:?}");
            assert!(report.findings.iter().any(|f| f.rule == "COMMENTEX-LINK" && f.message.contains("paraIdParent")), "{report:?}");
            assert!(!report.findings.iter().any(|f| f.message.contains("no commentsIds")), "{report:?}");
        }
    }
}

#[test]
fn every_relationship_reference_binds_the_first_duplicate_declaration() {
    use common::CT_HEADER;
    for image_first in [false, true] {
        let mut d = Doc::minimal();
        d.set_document(r#"<w:p xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><w:r><a:blip r:embed="same"/></w:r></w:p><w:sectPr><w:headerReference r:id="same"/></w:sectPr>"#);
        d.add_with_override("word/header.xml", CT_HEADER, format!(r#"<w:hdr {NS}><w:p/></w:hdr>"#));
        d.add_with_override("word/pic.png", "image/png", b"\x89PNG\r\n\x1a\n".to_vec());
        let header =
            r#"<Relationship Id="same" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header.xml"/>"#;
        let image = r#"<Relationship Id="same" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="pic.png"/>"#;
        d.set_doc_rels(&if image_first { format!("{image}{header}") } else { format!("{header}{image}") });
        let report = validate_bytes(&d.build());
        assert!(report.findings.iter().any(|f| f.rule == "REL-DUP-ID"), "{report:?}");
        assert_eq!(report.findings.iter().any(|f| f.rule == "REL-TYPED" && f.message.contains("an image element")), !image_first, "{report:?}");
        assert_eq!(report.findings.iter().any(|f| f.rule == "REF-HEADER"), image_first, "{report:?}");
        // ZIP order is immaterial. XML declaration order for duplicate bindings is evidence.
        d.entries.reverse();
        assert_eq!(serde_json::to_vec(&validate_bytes(&d.build())).unwrap(), serde_json::to_vec(&report).unwrap());
    }
}

#[test]
fn glossary_roots_keep_distinct_definitions_across_official_and_custom_edges() {
    use common::{CT_FOOTNOTES, CT_GLOSSARY, rels_xml};
    for matching in [false, true] {
        for edge in ["http://schemas.openxmlformats.org/officeDocument/2006/relationships/glossaryDocument", "urn:custom/glossaryDocument"] {
            let mut d = Doc::minimal();
            d.set_doc_rels(&rel("glossaryDocument", "a/document.xml"));
            for (folder, note, comment) in [("a", 2, 4), ("b", if matching { 2 } else { 3 }, if matching { 4 } else { 5 })] {
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
                let mut links = rel("footnotes", "footnotes.xml") + &rel("comments", "comments.xml");
                if folder == "a" {
                    links.push_str(&format!(r#"<Relationship Id="nested" Type="{edge}" Target="../b/document.xml"/>"#));
                }
                d.set(&format!("word/{folder}/_rels/document.xml.rels"), rels_xml(&links));
            }
            let report = report_in_all_orders(&mut d);
            assert_eq!(report.verdict, wordcraft_validate::Verdict::Pass, "{edge} matching={matching}: {report:?}");
            assert!(report.findings.is_empty(), "{report:?}");
        }
    }
}
