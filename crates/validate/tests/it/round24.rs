//! Round 24: an owner identity that several ZIP entries map to is ambiguous, so its facts never decide non-use.
use crate::common::*;
use crate::round20::raw_zip::{self, Record};
use wordcraft_validate::{Report, Verdict, validate_bytes};

const HEADER_RELS: &str = "word/_rels/header.xml.rels";

fn dangling_rels() -> Vec<u8> {
    rels_xml(r#"<Relationship Id="x" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="gone"/>"#)
}

fn build(extra: &[(&str, &[u8])]) -> Vec<u8> {
    let mut doc = Doc::minimal();
    let mut records: Vec<Record> = doc.entries.drain(..).map(|(n, b)| Record::new(n, 0, b)).collect();
    records.extend(extra.iter().map(|(n, b)| Record::new(n, 0, *b)));
    raw_zip::build(&records)
}

fn assessment_note(report: &Report) -> Option<&String> {
    report.coverage_notes.iter().find(|n| n.contains("relationship x") && n.contains("not decided"))
}

fn assert_undecided(report: &Report) {
    assert!(assessment_note(report).is_some(), "{report:?}");
    assert!(report.verdict != Verdict::Pass, "{report:?}");
}

#[test]
fn readable_case_variant_does_not_hide_an_undecodable_owner() {
    let rels = dangling_rels();
    // Control: the undecodable owner alone keeps the assessment note.
    assert_undecided(&validate_bytes(&build(&[("word/header.xml", b"\xff"), (HEADER_RELS, &rels)])));
    for extra in [
        [("word/header.xml", b"\xff".as_slice()), ("word/Header.xml", b"<root/>".as_slice())],
        [("word/Header.xml", b"<root/>".as_slice()), ("word/header.xml", b"\xff".as_slice())],
    ] {
        let mut entries = extra.to_vec();
        entries.push((HEADER_RELS, &rels));
        let report = validate_bytes(&build(&entries));
        assert!(report.findings.iter().any(|f| f.rule == "PKG-CASE-COLLISION"), "{report:?}");
        assert_undecided(&report);
    }
}

#[test]
fn case_collision_between_readable_owners_is_undecided() {
    let rels = dangling_rels();
    let report = validate_bytes(&build(&[("word/header.xml", b"<root/>"), ("word/Header.xml", b"<root/>"), (HEADER_RELS, &rels)]));
    assert_undecided(&report);
}

#[test]
fn duplicate_entries_under_one_name_are_undecided() {
    let rels = dangling_rels();
    let report = validate_bytes(&build(&[("word/header.xml", b"<root/>"), ("word/header.xml", b"<root/>"), (HEADER_RELS, &rels)]));
    assert!(report.findings.iter().any(|f| f.rule == "PKG-DUP-ENTRY"), "{report:?}");
    assert_undecided(&report);
}

#[test]
fn case_collision_in_the_relationships_part_leaves_its_owner_facts_usable_only_as_incomplete() {
    let rels = dangling_rels();
    let empty = rels_xml("");
    let report =
        validate_bytes(&build(&[("word/header.xml", b"<root/>"), ("word/_rels/header.xml.rels", &rels), ("word/_rels/Header.xml.rels", &empty)]));
    assert!(report.findings.iter().any(|f| f.rule == "PKG-CASE-COLLISION"), "{report:?}");
    assert_undecided(&report);
}
