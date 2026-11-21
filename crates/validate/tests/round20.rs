//! Round 20 binding H6 probes; raw metadata is patched only in synthetic ZIP headers.
#![allow(dead_code)]
mod common;
#[path = "common/raw_zip.rs"]
mod raw_zip;
use common::*;
use raw_zip::Record;
use std::io::{Cursor, Write};
use wordcraft_validate::{Report, Severity, Verdict, validate_bytes, validate_path};

// Deflated payloads from ZipWriter; unique placeholders let it write records that will collapse.
fn zipit(records: &[Record]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (index, r) in records.iter().enumerate() {
        let placeholder = format!("{index:0width$}", width = r.name.len());
        assert_eq!(placeholder.len(), r.name.len());
        let mut opts = zip::write::FileOptions::<zip::write::ExtendedFileOptions>::default().compression_method(zip::CompressionMethod::Deflated);
        if !r.extra.is_empty() {
            opts.add_extra_data(0xcafe, vec![0; r.extra.len() - 4].into_boxed_slice(), false).unwrap();
        }
        writer.start_file(placeholder, opts).unwrap();
        writer.write_all(&r.body).unwrap();
    }
    let mut bytes = writer.finish().unwrap().into_inner();
    for (sig, flags, nlen, elen, name_at) in [(b"PK\x03\x04", 6, 26, 28, 30), (b"PK\x01\x02", 8, 28, 30, 46)] {
        for i in 0..bytes.len().saturating_sub(name_at) {
            if bytes.get(i..i + 4) != Some(sig) {
                continue;
            }
            let n = u16::from_le_bytes(bytes[i + nlen..i + nlen + 2].try_into().unwrap()) as usize;
            let e = u16::from_le_bytes(bytes[i + elen..i + elen + 2].try_into().unwrap()) as usize;
            let name = &bytes[i + name_at..i + name_at + n];
            let index: usize = std::str::from_utf8(name).unwrap().parse().unwrap();
            let r = &records[index];
            bytes[i + flags..i + flags + 2].copy_from_slice(&r.flags.to_le_bytes());
            bytes[i + name_at..i + name_at + n].copy_from_slice(&r.name);
            assert_eq!(e, r.extra.len());
            bytes[i + name_at + n..i + name_at + n + e].copy_from_slice(&r.extra);
        }
    }
    bytes
}

fn records(d: Doc) -> Vec<Record> {
    d.entries
        .into_iter()
        .map(|(name, body)| {
            let flag = if name.is_ascii() { 0 } else { 0x800 };
            Record::new(name, flag, body)
        })
        .collect()
}
fn footnotes(ids: &[u32]) -> Vec<u8> {
    format!(
        r#"<w:footnotes {NS}>{}</w:footnotes>"#,
        ids.iter().map(|id| format!(r#"<w:footnote w:id="{id}"><w:p/></w:footnote>"#)).collect::<String>()
    )
    .into_bytes()
}
fn edge(id: &str, kind: &str, target: &str) -> String {
    format!(r#"<Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}" Target="{target}"/>"#)
}
fn note_doc(name: &str) -> Doc {
    let mut d = Doc::minimal();
    d.set_document(r#"<w:p><w:r><w:footnoteReference w:id="1"/></w:r></w:p>"#);
    d.set("[Content_Types].xml", content_types_xml(&[], &[(&format!("/{name}"), CT_FOOTNOTES)]));
    d.set_doc_rels(&edge("fn", "footnotes", name.strip_prefix("word/").unwrap()));
    d
}
fn count(report: &Report, rule: &str) -> usize {
    report.findings.iter().filter(|f| f.rule == rule).count()
}
fn exact_rules(report: &Report, rules: &[&str]) {
    let mut actual: Vec<_> = report.findings.iter().map(|f| f.rule).collect();
    actual.sort_unstable();
    let mut expected = rules.to_vec();
    expected.sort_unstable();
    assert_eq!(actual, expected, "{report:?}");
}
// Generated policy is included read-only to compare the complete absence-rule surface.
#[path = "../src/contracts_generated.rs"]
mod policy;
fn stable(a: &Report, b: &Report) {
    assert_eq!(a.verdict, b.verdict);
    let groups = |r: &Report| {
        let mut v: Vec<_> = r.findings.iter().filter(|f| f.rule == "PKG-DUP-ENTRY").map(|f| f.message.clone()).collect();
        v.sort();
        v
    };
    let absences = |r: &Report| {
        let mut v: Vec<_> = r
            .findings
            .iter()
            .filter(|f| policy::PACKAGE_ABSENCE_RULES.contains(&f.rule))
            .map(|f| (f.rule, f.part.clone(), f.message.clone()))
            .collect();
        v.sort();
        v
    };
    assert_eq!(groups(a), groups(b));
    assert_eq!(absences(a), absences(b));
    assert_eq!(a.checks_run, b.checks_run);
}
fn duplicate_orders(base: Vec<Record>, pair: Vec<Record>, keys: &[&str], extra: &[&str]) -> [Report; 2] {
    let reports = [false, true].map(|reverse| {
        let all: Vec<_> = base
            .iter()
            .chain(if reverse { pair.iter().rev().collect::<Vec<_>>() } else { pair.iter().collect() })
            .map(|r| Record { name: r.name.clone(), flags: r.flags, extra: r.extra.clone(), body: r.body.clone() })
            .collect();
        let report = validate_bytes(&zipit(&all));
        assert_eq!(report.verdict, Verdict::Fail, "{report:?}");
        let mut expected = vec!["PKG-DUP-ENTRY"; keys.len()];
        expected.extend_from_slice(extra);
        exact_rules(&report, &expected);
        assert!(report.checks_run.contains(&"PKG-DUP-ENTRY"));
        for key in keys {
            let f: Vec<_> = report
                .findings
                .iter()
                .filter(|f| f.rule == "PKG-DUP-ENTRY" && f.message == format!("the entry {key} appears more than once in the archive"))
                .collect();
            assert_eq!(f.len(), 1);
            assert_eq!(f[0].part, None);
            assert_eq!(f[0].severity, Severity::Error);
            assert!(report.coverage_notes.iter().any(|n| n.contains(key) && n.contains("not examined")));
        }
        report
    });
    stable(&reports[0], &reports[1]);
    reports
}
fn fn_pair(first: Record, second: Record, name: &str) -> [Report; 2] {
    let reports = duplicate_orders(records(note_doc(name)), vec![first, second], &[name], &[]);
    for report in &reports {
        assert!(!report.checks_run.contains(&"REF-NOTE"));
    }
    reports
}

#[test]
fn h6_p1_p7_p13_decoded_and_raw_note_duplicates_both_orders_and_path() {
    for (name, second, flag) in [("word/fü.xml", b"word/f\x81.xml".as_slice(), 0), ("word/footnotes.xml", b"word/footnotes.xml".as_slice(), 0)] {
        let first = Record::new(name, if name.is_ascii() { 0 } else { 0x800 }, footnotes(&[1]));
        let second = Record::new(second, flag, footnotes(&[]));
        let base = records(note_doc(name));
        let reports = duplicate_orders(base, vec![first, second], &[name], &[]);
        for report in &reports {
            assert!(!report.checks_run.contains(&"REF-NOTE"));
        }
        for reverse in [false, true] {
            let mut all = records(note_doc(name));
            let mut pair = vec![
                Record::new(name, if name.is_ascii() { 0 } else { 0x800 }, footnotes(&[1])),
                Record::new(if name.is_ascii() { name.as_bytes() } else { b"word/f\x81.xml" }, 0, footnotes(&[])),
            ];
            if reverse {
                pair.reverse();
            }
            all.extend(pair);
            let path = std::env::temp_dir().join(format!("wordcraft-r20-{}-{reverse}-{}.docx", std::process::id(), name.len()));
            std::fs::write(&path, zipit(&all)).unwrap();
            let report = validate_path(&path).unwrap();
            std::fs::remove_file(path).unwrap();
            stable(&report, &reports[usize::from(reverse)]);
            exact_rules(&report, &["PKG-DUP-ENTRY"]);
        }
    }
}
#[test]
fn h6_p1l_lossy_names_and_literal_replacement_collapse() {
    fn_pair(Record::new(b"word/a\xff\xff.xml", 0x800, footnotes(&[1])), Record::new(b"word/a\xfe\xfe.xml", 0x800, footnotes(&[])), "word/a��.xml");
    fn_pair(Record::new("word/a�.xml", 0x800, footnotes(&[1])), Record::new(b"word/a\xff.xml", 0x800, footnotes(&[])), "word/a�.xml");
}
#[test]
fn h6_p2_unicode_path_duplicate_and_rejection_controls() {
    fn_pair(
        Record::new("word/fα.xml", 0x800, footnotes(&[1])),
        Record::new("word/fB.xml", 0, footnotes(&[])).unicode(1, b"word/fB.xml", "word/fα.xml".as_bytes()),
        "word/fα.xml",
    );
    let mut bad = Record::new("word/fB.xml", 0, footnotes(&[])).unicode(1, b"word/fB.xml", "word/fα.xml".as_bytes());
    bad.extra[5] ^= 1;
    for bad in [bad, Record::new("word/fB.xml", 0, footnotes(&[])).unicode(1, b"word/fB.xml", b"\xff")] {
        for reverse in [false, true] {
            let mut base = records(note_doc("word/fα.xml"));
            let good = Record::new("word/fα.xml", 0x800, footnotes(&[1]));
            if reverse {
                base.extend([Record::new(&bad.name, bad.flags, &bad.body).unicode(1, b"word/fB.xml", b"unused"), good]);
                base[4].extra = bad.extra.clone();
            } else {
                base.extend([good, Record::new(&bad.name, bad.flags, &bad.body)]);
                base[5].extra = bad.extra.clone();
            }
            let report = validate_bytes(&zipit(&base));
            assert_eq!(report.verdict, Verdict::Fail);
            exact_rules(&report, &["PKG-NOT-ZIP"]);
            assert_eq!(report.checks_run, ["PKG-NOT-ZIP"]);
        }
    }
}
#[test]
fn h6_negative_controls_p1n_p1o_p1u_p2d_p4_p5_p6_p11() {
    let empty = b"<root/>";
    let pairs = vec![
        (Record::new(b"word/f\x80.xml", 0, empty), Record::new(b"word/f\x81.xml", 0, empty)),
        (Record::new("word/fü.xml", 0x800, empty), Record::new("word/fé.xml", 0x800, empty)),
        (Record::new("word/a�.xml", 0x800, empty), Record::new(b"word/a\xff\xff.xml", 0x800, empty)),
        (Record::new("word/fα.xml", 0x800, empty), Record::new("word/fB.xml", 0, empty).unicode(1, b"word/fB.xml", "word/fβ.xml".as_bytes())),
        (Record::new("word/f.xml", 0, empty), Record::new("word\\f.xml", 0, empty)),
        (Record::new("word/é.xml", 0x800, empty), Record::new("word/e\u{301}.xml", 0x800, empty)),
        (Record::new("word/ü.xml", 0x800, empty), Record::new("word/Ü.xml", 0x800, empty)),
    ];
    for (a, b) in pairs {
        for reverse in [false, true] {
            let mut base = records(Doc::minimal());
            let pair = if reverse { [&b, &a] } else { [&a, &b] };
            base.extend(pair.map(|r| Record { name: r.name.clone(), flags: r.flags, extra: r.extra.clone(), body: r.body.clone() }));
            let report = validate_bytes(&zipit(&base));
            assert_eq!(report.verdict, Verdict::Pass, "{report:?}");
            exact_rules(&report, &[]);
            assert!(report.checks_run.contains(&"PKG-DUP-ENTRY"));
        }
    }
    for reverse in [false, true] {
        let mut base = records(Doc::minimal());
        let mut pair = vec![Record::new("word/a.xml", 0, empty), Record::new("word/b.xml", 0, empty)];
        if reverse {
            pair.reverse();
        }
        base.extend(pair);
        let mut bytes = zipit(&base);
        for i in 0..bytes.len() - 40 {
            if bytes.get(i..i + 4) == Some(b"PK\x03\x04") && bytes.get(i + 30..i + 40) == Some(b"word/b.xml") {
                bytes[i + 35] = b'a';
            }
        }
        let report = validate_bytes(&bytes);
        assert_eq!(report.verdict, Verdict::Pass, "{report:?}");
        exact_rules(&report, &[]);
        assert!(report.checks_run.contains(&"PKG-DUP-ENTRY"));
    }
}
#[test]
fn h6_p3_leading_slash_and_directory_duplicates() {
    duplicate_orders(
        records(Doc::minimal()),
        vec![Record::new("/word/f.xml", 0, b"<r/>"), Record::new("word/f.xml", 0, b"<r/>")],
        &["word/f.xml"],
        &[],
    );
    duplicate_orders(records(Doc::minimal()), vec![Record::new("word/x/", 0, b""), Record::new("word/x/", 0, b"")], &["word/x/"], &[]);
}
#[test]
fn h6_p8_combined_groups() {
    let reports = duplicate_orders(
        records(note_doc("word/fü.xml")),
        vec![
            Record::new("word/fü.xml", 0x800, footnotes(&[1])),
            Record::new(b"word/f\x81.xml", 0, footnotes(&[])),
            Record::new("word/g.xml", 0, b"<r/>"),
            Record::new("word/g.xml", 0, b"<r/>"),
        ],
        &["word/fü.xml", "word/g.xml"],
        &[],
    );
    for report in reports {
        assert!(!report.checks_run.contains(&"REF-NOTE"));
    }
}
#[test]
fn h6_p14_collapse_keeps_unrelated_style_absence() {
    let mut d = note_doc("word/fü.xml");
    d.set_document(r#"<w:p><w:pPr><w:pStyle w:val="NoSuch"/></w:pPr><w:r><w:footnoteReference w:id="1"/></w:r></w:p>"#);
    // Amendment 3: a reachable complete styles source that lacks NoSuch.
    d.add_with_override("word/styles.xml", CT_STYLES, format!(r#"<w:styles {NS}/>"#));
    d.set_doc_rels(&(edge("fn", "footnotes", "fü.xml") + &edge("styles", "styles", "styles.xml")));
    let reports = duplicate_orders(
        records(d),
        vec![Record::new("word/fü.xml", 0x800, footnotes(&[1])), Record::new(b"word/f\x81.xml", 0, footnotes(&[]))],
        &["word/fü.xml"],
        &["REF-STYLE"],
    );
    for report in reports {
        assert!(report.checks_run.contains(&"REF-STYLE"));
        assert!(!report.checks_run.contains(&"REF-NOTE"));
        assert_eq!(report.findings.iter().find(|f| f.rule == "REF-STYLE").unwrap().part.as_deref(), Some("word/document.xml"));
    }
}
#[test]
fn h6_p15_duplicate_package_rels_unproven_main() {
    let mut d = Doc::minimal();
    d.remove("_rels/.rels");
    let reports = duplicate_orders(
        records(d),
        vec![Record::new("_rels/.rels", 0, package_rels_xml()), Record::new("_rels/.rels", 0, rels_xml(""))],
        &["_rels/.rels"],
        &[],
    );
    for report in reports {
        assert!(report.checks_run.contains(&"PKG-NO-MAIN"));
        assert!(report.coverage_notes.iter().any(|n| n.contains("main is unproven") && n.contains("_rels/.rels")));
    }
}
#[test]
fn h6_p10_malformed_main_survivor_keeps_positive_finding() {
    let mut d = Doc::minimal();
    d.remove("word/document.xml");
    d.remove("word/_rels/document.xml.rels");
    d.set("[Content_Types].xml",format!(r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/word/dü.xml" ContentType="{CT_MAIN}"/></Types>"#));
    d.set("_rels/.rels", rels_xml(&edge("main", "officeDocument", "word/dü.xml")));
    let good = document_xml("<w:p/>");
    let bad = format!(r#"<w:document {NS}><w:body><w:p>"#);
    let reports = [false, true].map(|reverse| {
        let mut base = records(Doc { entries: d.entries.clone() });
        let mut pair = vec![Record::new("word/dü.xml", 0x800, &good), Record::new(b"word/d\x81.xml", 0, bad.as_bytes())];
        if reverse {
            pair.reverse();
        }
        base.extend(pair);
        let report = validate_bytes(&zipit(&base));
        assert_eq!(report.verdict, Verdict::Fail);
        exact_rules(&report, if reverse { &["PKG-DUP-ENTRY"] } else { &["PKG-DUP-ENTRY", "XML-MALFORMED"] });
        assert!(report.checks_run.contains(&"XML-MALFORMED"));
        if !reverse {
            assert_eq!(report.findings.iter().find(|f| f.rule == "XML-MALFORMED").unwrap().part.as_deref(), Some("word/dü.xml"));
        }
        report
    });
    stable(&reports[0], &reports[1]);
}

#[test]
fn h6_p9_entry_cap_does_not_invent_suffix_duplicates() {
    let mut d = Doc::minimal();
    d.set("[Content_Types].xml", content_types_xml(&[("bin", "application/octet-stream")], &[]));
    for i in 0..20_000 {
        d.set(&format!("filler/{i}.bin"), Vec::new());
    }
    let report = validate_bytes(&d.build());
    assert_eq!(report.verdict, Verdict::Incomplete, "{report:?}");
    exact_rules(&report, &[]);
    assert!(report.checks_run.contains(&"PKG-DUP-ENTRY"));
    assert!(report.coverage_notes.iter().any(|n| n.contains("later names were not checked")));
    assert!(!report.coverage_notes.iter().any(|n| n.contains("disagrees with the reader")));
}
