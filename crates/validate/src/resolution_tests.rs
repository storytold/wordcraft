//! Resolution controls exercise discovery states independently of report gates.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "../tests/common/mod.rs"]
pub(super) mod common;
use common::{CT_STYLES, Doc, NS};

#[test]
fn empty_incomplete_discovery_is_unknown() {
    let mut pkg = package::load(&Doc::minimal().build(), &mut Acc::with_limits(Limits::default())).unwrap();
    assert!(matches!(owner_rel_part(&pkg, "word/document.xml", "styles"), Resolution::Absent));
    pkg.entries_truncated = true;
    assert!(matches!(owner_rel_part(&pkg, "word/document.xml", "styles"), Resolution::Unknown(_)));
    assert!(matches!(find_by_type(&pkg, CT_STYLES), Resolution::Unknown(_)));
    pkg.entries_truncated = false;
    pkg.identity_incomplete = true;
    assert!(matches!(owner_rel_part(&pkg, "word/document.xml", "styles"), Resolution::Unknown(_)));
    assert!(matches!(find_by_type(&pkg, CT_STYLES), Resolution::Unknown(_)));
}

#[test]
fn content_type_singletons_never_choose_by_zip_order() {
    for reverse in [false, true] {
        let mut d = Doc::minimal();
        for name in ["word/a.xml", "word/b.xml"] {
            d.add_with_override(name, CT_STYLES, format!(r#"<w:styles {NS}/>"#));
        }
        if reverse {
            d.entries.reverse();
        }
        let pkg = package::load(&d.build(), &mut Acc::with_limits(Limits::default())).unwrap();
        assert!(matches!(find_by_type(&pkg, CT_STYLES), Resolution::Ambiguous(_)));
    }
}

#[test]
fn unreadable_root_discovery_never_establishes_absence() {
    let mut d = Doc::minimal();
    d.set("_rels/.rels", vec![0xff]);
    let pkg = package::load(&d.build(), &mut Acc::with_limits(Limits::default())).unwrap();
    assert!(matches!(pkg.main, Resolution::Unknown(_)));
    assert!(matches!(package::entry_point_profile(&pkg), Resolution::Unknown(_)));
}

/// R18/45: identity loss never erases a found prefix or establishes missing identity.
#[test]
fn round18_entry_lookup_truth_table() {
    for reverse in [false, true] {
        let mut d = Doc::minimal();
        d.add_with_override("word/Header.XML", common::CT_HEADER, format!(r#"<w:hdr {NS}><w:p/></w:hdr>"#));
        d.set_doc_rels(
            r#"<Relationship Id="h" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="Header.XML"/>"#,
        );
        if reverse {
            d.entries.reverse();
        }
        for (truncated, identity) in [(false, false), (true, false), (false, true), (true, true)] {
            let mut pkg = package::load(&d.build(), &mut Acc::with_limits(Limits::default())).unwrap();
            pkg.entries_truncated = truncated;
            pkg.identity_incomplete = identity;
            assert!(matches!(canonical_entry(&pkg, "/WORD/header.xml"), Resolution::One(n) if n == "word/Header.XML"));
            assert!(matches!(entry_by_name(&pkg, "/WORD/header.xml"), Resolution::One(e) if e.name == "word/Header.XML"));
            assert!(matches!(package::content_unavailable(&pkg, "word/Header.XML"), Resolution::One(false)));
            let missing = canonical_entry(&pkg, "word/missing.xml");
            let unavailable = package::content_unavailable(&pkg, "word/missing.xml");
            if truncated || identity {
                assert!(matches!(missing, Resolution::Unknown(_)));
                assert!(matches!(unavailable, Resolution::Unknown(_)));
            } else {
                assert!(matches!(missing, Resolution::Absent));
                assert!(matches!(unavailable, Resolution::Absent));
            }
            // Positive role classification survives a truncated prefix.
            assert_eq!(story_roles(&pkg).get("word/header.xml"), Some(&scan::PartRole::Story));
            let mut acc = Acc::with_limits(Limits::default());
            let xml = check_xml(&pkg, &mut acc);
            check_rels(&pkg, &xml, &mut acc);
            assert!(!acc.finish().findings.iter().any(|f| f.rule == "REL-DANGLING"));
            pkg.entries.retain(|e| !e.name.eq_ignore_ascii_case("word/Header.XML"));
            let mut acc = Acc::with_limits(Limits::default());
            let xml = check_xml(&pkg, &mut acc);
            check_rels(&pkg, &xml, &mut acc);
            assert_eq!(acc.finish().findings.iter().any(|f| f.rule == "REL-DANGLING"), !truncated && !identity);
        }
    }
}

/// R18/32: the retained whole-source gates are conservative and necessary independently.
#[test]
fn round18_discovery_loss_polarity() {
    for (truncated, identity) in [(false, false), (true, false), (false, true)] {
        let mut pkg = package::load(&Doc::minimal().build(), &mut Acc::with_limits(Limits::default())).unwrap();
        pkg.entries_truncated = truncated;
        pkg.identity_incomplete = identity;
        assert_eq!(discovery_unavailable(&pkg, None), truncated || identity);
        assert_eq!(discovery_unavailable(&pkg, Some("word/document.xml")), truncated || identity);
        assert_eq!(ownership_discovery_complete(&pkg), !truncated && !identity);
    }
}

/// R18/46: each availability consumer has separate readable, unavailable, missing and unknown controls.
#[test]
fn round18_content_availability_sites() {
    for reverse in [false, true] {
        for state in ["readable", "unavailable", "absent", "unknown"] {
            let mut d = Doc::minimal();
            d.set_document(r#"<w:p><w:r><w:drawing><a:blip xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" r:embed="h"/></w:drawing></w:r></w:p>"#);
            d.set_doc_rels(r#"<Relationship Id="h" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="image.xml"/><Relationship Id="header" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="image.xml"/>"#);
            d.add_with_override("word/image.xml", common::CT_STYLES, format!(r#"<w:styles {NS}/>"#));
            if reverse {
                d.entries.reverse();
            }
            let mut pkg = package::load(&d.build(), &mut Acc::with_limits(Limits::default())).unwrap();
            match state {
                "unavailable" => {
                    let e = pkg.entries.iter_mut().find(|e| e.name == "[Content_Types].xml").unwrap();
                    e.content_unavailable = true;
                    e.decoded = None;
                    pkg.content_types.incomplete = true;
                }
                "absent" | "unknown" => {
                    pkg.entries.retain(|e| e.name != "[Content_Types].xml");
                    pkg.content_types_present = false;
                    pkg.entries_truncated = state == "unknown";
                    pkg.content_types.incomplete = state == "unknown";
                }
                _ => {}
            }
            let unavailable = package::content_unavailable(&pkg, "[Content_Types].xml");
            match state {
                "readable" => assert!(matches!(unavailable, Resolution::One(false))),
                "unavailable" => assert!(matches!(unavailable, Resolution::One(true))),
                "absent" => assert!(matches!(unavailable, Resolution::Absent)),
                _ => assert!(matches!(unavailable, Resolution::Unknown(_))),
            }
            let skip = matches!(state, "unavailable" | "unknown");
            assert_eq!(discovery_unavailable(&pkg, None), skip, "2756/{state}");
            assert_eq!(ownership_discovery_complete(&pkg), !skip, "2780/{state}");
            let mut acc = Acc::with_limits(Limits::default());
            check_content_types(&pkg, &mut acc);
            assert_eq!(acc.checks.contains(&"CT-UNDECLARED"), state == "readable", "905/{state}");
            let xml = check_xml(&pkg, &mut acc);
            let mut acc = Acc::with_limits(Limits::default());
            check_typed_image_refs(&pkg, &xml.references(), &mut acc);
            assert_eq!(acc.findings.iter().any(|f| f.rule == "REL-TYPED"), !skip, "2034/{state}");
            let mut acc = Acc::with_limits(Limits::default());
            check_typed_targets(&pkg, &mut acc);
            assert_eq!(acc.findings.iter().any(|f| f.rule == "REL-TYPED"), !skip, "2097/{state}");
            // Site 3650's manifest-driven note belongs to the explicit no-owner fallback.
            pkg.main = Resolution::Absent;
            pkg.glossary = Resolution::Absent;
            let mut acc = Acc::with_limits(Limits::default());
            let xml = check_xml(&pkg, &mut acc);
            let mut acc = Acc::with_limits(Limits::default());
            check_custom_xml(&pkg, &xml, &mut acc);
            assert!(!acc.findings.iter().any(|f| f.rule == "CUSTOMXML-SCHEMA"), "3650/{state}");
            assert_eq!(
                acc.coverage.iter().any(|n| n.contains("custom XML schema references were not checked")),
                state == "unavailable",
                "3650 informational/{state}"
            );
        }
    }
}

/// R18/47: actual ZIP limits move a target between the examined prefix and suffix.
#[test]
fn round18_archive_prefix_permutations() {
    let mut d = Doc::minimal();
    d.add_with_override("word/header.xml", common::CT_HEADER, format!(r#"<w:hdr {NS}><w:p/></w:hdr>"#));
    d.set_doc_rels(r#"<Relationship Id="h" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header.xml"/>"#);
    let complete = serde_json::to_vec(&validate_bytes(&d.build())).unwrap();
    let entries = d.entries.clone();
    for shift in 0..entries.len() {
        d.entries = entries.clone();
        d.entries.rotate_left(shift);
        assert_eq!(serde_json::to_vec(&validate_bytes(&d.build())).unwrap(), complete);
        for cap in 0..=entries.len() {
            let mut acc = Acc::with_limits(Limits { entries: cap, ..Limits::default() });
            let pkg = package::load(&d.build(), &mut acc).unwrap();
            let examined = d.entries.iter().take(cap).any(|(name, _)| name == "word/header.xml");
            assert_eq!(matches!(canonical_entry(&pkg, "word/header.xml"), Resolution::One(_)), examined);
            if !examined {
                assert!(matches!(canonical_entry(&pkg, "word/header.xml"), Resolution::Unknown(_)));
            }
            let report = validate_bytes_limited(&d.build(), Limits { entries: cap, ..Limits::default() });
            assert!(!report.findings.iter().any(|f| f.rule == "REL-DANGLING"), "shift={shift}/cap={cap}: {report:?}");
        }
    }
}

/// R18/2304: the name lookup only supplies display spelling to an independent typed finding.
#[test]
fn round18_typed_finding_display_polarity() {
    for state in ["found", "absent", "unknown"] {
        let mut d = Doc::minimal();
        d.add_with_override("word/Mixed.XML", common::CT_STYLES, format!(r#"<w:styles {NS}/>"#));
        d.set_doc_rels(
            r#"<Relationship Id="c" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="mixed.xml"/>"#,
        );
        let mut pkg = package::load(&d.build(), &mut Acc::with_limits(Limits::default())).unwrap();
        if state != "found" {
            pkg.entries.retain(|e| e.name != "word/Mixed.XML");
        }
        pkg.entries_truncated = state == "unknown";
        let mut acc = Acc::with_limits(Limits::default());
        let xml = check_xml(&pkg, &mut acc);
        let mut acc = Acc::with_limits(Limits::default());
        check_typed_content(&pkg, &xml, &mut acc);
        let report = acc.finish();
        let finding = report.findings.iter().find(|f| f.rule == "CT-MISMATCH").unwrap();
        assert_eq!(finding.part.as_deref(), Some(if state == "found" { "word/Mixed.XML" } else { "word/mixed.xml" }), "{state}");
    }
}

/// CodeRabbit PR#18: proven REL-TYPED mismatches survive unknown_unreadable, while absence stays suppressed.
#[test]
fn rel_typed_preserves_proven_mismatches_under_unknown_unreadable() {
    const A_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
    let mut d = Doc::minimal();
    d.set(
        "word/document.xml",
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document {NS} xmlns:a="{A_NS}"><w:body><w:p>
  <a:blip r:embed="rImgNonImage"/>
  <a:blip r:embed="rImgExternal"/>
  <a:blip r:embed="rImgWrongCt"/>
  <a:blip r:embed="rImgNoCt"/>
</w:p></w:body></w:document>"#
        ),
    );
    d.add_with_override("word/header1.xml", "application/xml", r#"<raw/>"#);
    d.add_with_override("word/img_wrong.dat", "application/xml", b"raw".to_vec());
    d.set("word/img_missing.undeclared", b"raw".to_vec());
    d.set_doc_rels(
        r#"
  <Relationship Id="rHdrWrongCt" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/>
  <Relationship Id="rHdrNoCt" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="img_missing.undeclared"/>
  <Relationship Id="rImgNonImage" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="header1.xml"/>
  <Relationship Id="rImgExternal" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="https://example.com/a.png" TargetMode="External"/>
  <Relationship Id="rImgWrongCt" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="img_wrong.dat"/>
  <Relationship Id="rImgNoCt" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="img_missing.undeclared"/>
"#,
    );
    let pkg = package::load(&d.build(), &mut Acc::with_limits(Limits::default())).unwrap();
    let mut scan_acc = Acc::with_limits(Limits::default());
    let xml = check_xml(&pkg, &mut scan_acc);

    let mut acc = Acc::with_limits(Limits::default());
    acc.set_unknown_unreadable();
    check_typed_targets(&pkg, &mut acc);
    let image_refs: Vec<(String, Vec<RefFact>)> = xml.parts.values().map(|p| (p.name.clone(), p.references.prefix().to_vec())).collect();
    check_typed_image_refs(&pkg, &image_refs, &mut acc);
    let report = acc.finish();

    let messages: Vec<&str> = report.findings.iter().filter(|f| f.rule == "REL-TYPED").map(|f| f.message.as_str()).collect();
    assert!(messages.iter().any(|m| m.contains("rHdrWrongCt")), "expected header ct mismatch: {messages:?}");
    assert!(!messages.iter().any(|m| m.contains("rHdrNoCt")), "expected absence finding suppressed: {messages:?}");
    assert!(messages.iter().any(|m| m.contains("rImgNonImage")), "expected non-image role mismatch: {messages:?}");
    assert!(messages.iter().any(|m| m.contains("rImgExternal")), "expected external r:embed mismatch: {messages:?}");
    assert!(messages.iter().any(|m| m.contains("rImgWrongCt")), "expected image ct mismatch: {messages:?}");
    assert!(!messages.iter().any(|m| m.contains("rImgNoCt")), "expected image absence finding suppressed: {messages:?}");
}

/// CodeRabbit PR#18: capped retained refs still check against complete owned defs and mark skipped.
#[test]
fn check_owned_ids_retained_refs_fire_when_refs_capped() {
    const CT_COMMENTS: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml";
    let mut d = Doc::minimal();
    d.set(
        "word/document.xml",
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document {NS}><w:body><w:p>
  <w:r><w:commentReference w:id="99"/></w:r>
  <w:r><w:commentReference w:id="1"/></w:r>
  <w:r><w:commentReference w:id="2"/></w:r>
</w:p></w:body></w:document>"#
        ),
    );
    d.add_with_override("word/comments.xml", CT_COMMENTS, format!(r#"<w:comments {NS}><w:comment w:id="1"><w:p/></w:comment></w:comments>"#));
    d.set_doc_rels(
        r#"<Relationship Id="rCom" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/>"#,
    );

    let report = validate_bytes_limited(&d.build(), Limits { facts: 2, ..Limits::default() });
    assert!(
        report.findings.iter().any(|f| f.rule == "ID-COMMENT-REF" && f.message.contains("99")),
        "expected ID-COMMENT-REF for retained dangling id 99: {report:?}"
    );
    assert_eq!(report.verdict, Verdict::Fail, "{report:?}");
    assert!(report.coverage_notes.iter().any(|n| n.contains("fact/reference budget")), "{report:?}");

    // When all retained refs are valid and refs are capped, coverage stays incomplete.
    d.set(
        "word/document.xml",
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document {NS}><w:body><w:p>
  <w:r><w:commentReference w:id="1"/></w:r>
  <w:r><w:commentReference w:id="1"/></w:r>
  <w:r><w:commentReference w:id="1"/></w:r>
</w:p></w:body></w:document>"#
        ),
    );
    let clean_report = validate_bytes_limited(&d.build(), Limits { facts: 2, ..Limits::default() });
    assert_eq!(clean_report.verdict, Verdict::Incomplete, "{clean_report:?}");
}

/// CodeRabbit PR#18: failing decompression still charges partially read bytes against total_bytes.
#[test]
fn failed_decompression_counts_partial_bytes_against_total_budget() {
    use std::io::Write;
    let payload_bad = vec![b'A'; 400];
    let payload_next = vec![b'B'; 200];
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    w.start_file("word/bad.xml", opts).unwrap();
    w.write_all(&payload_bad).unwrap();
    w.start_file("word/next.xml", opts).unwrap();
    w.write_all(&payload_next).unwrap();
    let mut raw = w.finish().unwrap().into_inner();
    let pos = raw.windows(8).position(|s| s == b"AAAAAAAA").expect("stored payload");
    raw[pos] = b'Z';

    let mut acc = Acc::with_limits(Limits { part_bytes: 500, total_bytes: 500, ..Limits::default() });
    let _ = package::load(&raw, &mut acc);
    let report = acc.finish();
    assert!(report.coverage_notes.iter().any(|n| n.contains("could not be decompressed")), "expected bad entry decompression note: {report:?}");
    assert!(report.coverage_notes.iter().any(|n| n.contains("total limit")), "expected total budget enforcement after partial read: {report:?}");
}
