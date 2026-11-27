//! PKG-CONCATENATED: only a verified archive footer bounds the scan, and an invalid content-types inventory
//! cannot certify a standalone package.
use crate::common::*;
use crate::round20::raw_zip::{self, Record};
use wordcraft_validate::{Report, Verdict, validate_bytes};

const NOTE: &str = "could not resolve a candidate boundary";

fn concatenated(report: &Report) -> bool {
    report.findings.iter().any(|f| f.rule == "PKG-CONCATENATED")
}

fn noted(report: &Report) -> bool {
    report.coverage_notes.iter().any(|n| n.contains(NOTE))
}

fn records(doc: Doc) -> Vec<Record> {
    doc.entries.into_iter().map(|(name, body)| Record::new(name, 0, body)).collect()
}

/// The footer record of a built archive: (footer offset, first central record).
fn first_central_record(zip: &[u8]) -> Vec<u8> {
    let footer = zip.len() - 22;
    let start = u32::from_le_bytes(zip[footer + 16..footer + 20].try_into().unwrap()) as usize;
    let name = u16::from_le_bytes(zip[start + 28..start + 30].try_into().unwrap()) as usize;
    zip[start..start + 46 + name].to_vec()
}

#[derive(Clone, Copy)]
enum Fake {
    /// A central header signature followed by zeros.
    Zeroed,
    /// A copy of the real first central record: well formed, but the entries do not tile up to it.
    Copied,
}

/// A first archive whose entry data holds an EOCD-like record declaring a `long` comment that
/// ends `tail` bytes before the end of the `rest`-extended file; returns (first archive, whole file).
fn with_fake_footer(fake: Fake, rest: &[u8], tail: i64) -> (Vec<u8>, Vec<u8>) {
    let plain = raw_zip::build(&records(Doc::minimal()));
    let central = match fake {
        Fake::Zeroed => [b"PK\x01\x02".to_vec(), vec![0; 42]].concat(),
        Fake::Copied => first_central_record(&plain),
    };
    let make = |start: u32, comment: u16| {
        let mut blob = central.clone();
        blob.extend(*b"PK\x05\x06");
        for value in [0u16, 0, 1, 1] {
            blob.extend(value.to_le_bytes());
        }
        blob.extend((central.len() as u32).to_le_bytes());
        blob.extend(start.to_le_bytes());
        blob.extend(comment.to_le_bytes());
        blob
    };
    let build = |blob: &[u8]| {
        let mut recs = records(Doc::minimal());
        recs.push(Record::new("word/media/blob.bin", 0, blob));
        raw_zip::build(&recs)
    };
    let probe = build(&make(0, 0));
    // The blob is the last entry; its data starts right after its local header's name.
    let name = b"word/media/blob.bin";
    let blob_at = probe.windows(name.len()).position(|w| w == name).unwrap() + name.len();
    let fake_footer = blob_at + central.len();
    let total = (probe.len() + rest.len()) as i64;
    let comment = u16::try_from(total - (fake_footer as i64 + 22) - tail).unwrap();
    let first = build(&make(blob_at as u32, comment));
    assert_eq!(first.len(), probe.len());
    let whole = [first.as_slice(), rest].concat();
    (first, whole)
}

#[test]
fn a_footer_copied_into_entry_data_does_not_hide_a_real_concatenation() {
    let second = Doc::minimal().build();
    for fake in [Fake::Zeroed, Fake::Copied] {
        // The fake comment runs past the real first footer, into the second archive.
        let (_, whole) = with_fake_footer(fake, &second, 5);
        let report = validate_bytes(&whole);
        assert!(concatenated(&report), "{report:?}");
        assert_eq!(report.verdict, Verdict::Fail, "{report:?}");
    }
}

#[test]
fn a_fake_footer_comment_ending_at_the_real_boundary_is_never_silent() {
    let second = Doc::minimal().build();
    for fake in [Fake::Zeroed, Fake::Copied] {
        let (first, whole) = with_fake_footer(fake, &second, second.len() as i64);
        assert_eq!(whole.len(), first.len() + second.len());
        let report = validate_bytes(&whole);
        assert!(concatenated(&report) || noted(&report), "{report:?}");
        assert_ne!(report.verdict, Verdict::Pass, "{report:?}");
    }
}

#[test]
fn a_fake_footer_comment_short_of_the_real_footer_is_never_silent() {
    let second = Doc::minimal().build();
    for fake in [Fake::Zeroed, Fake::Copied] {
        let (first, whole) = with_fake_footer(fake, &second, (second.len() + 40) as i64);
        assert!(whole.len() > first.len());
        let report = validate_bytes(&whole);
        assert!(concatenated(&report) || noted(&report), "{report:?}");
        assert_ne!(report.verdict, Verdict::Pass, "{report:?}");
    }
}

#[test]
fn a_fake_footer_alone_is_not_a_concatenation() {
    for fake in [Fake::Zeroed, Fake::Copied] {
        let (first, _) = with_fake_footer(fake, &[], 0);
        let report = validate_bytes(&first);
        assert!(!concatenated(&report), "{report:?}");
    }
}

fn archive_with_content_types(content_types: &str) -> Vec<u8> {
    let mut doc = Doc::minimal();
    doc.set("[Content_Types].xml", content_types);
    doc.build()
}

fn valid_content_types() -> String {
    String::from_utf8(content_types_xml(&[], &[])).unwrap()
}

#[test]
fn an_invalid_content_types_inventory_does_not_certify_a_second_archive() {
    let valid = valid_content_types();
    let ordinary = Doc::minimal().build();
    let control = validate_bytes(&[ordinary.as_slice(), ordinary.as_slice()].concat());
    assert!(concatenated(&control), "{control:?}");

    let second_root = format!(r#"{valid}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#);
    let doctype = valid.replacen("?><Types", "?><!DOCTYPE Types><Types", 1);
    let dashes = valid.replace("</Types>", "<!-- a -- b --></Types>");
    for (label, ct) in [("second root", second_root), ("doctype", doctype), ("double hyphen in a comment", dashes)] {
        assert_ne!(ct, valid, "{label}");
        let bad = archive_with_content_types(&ct);
        for (first, second) in [(&ordinary, &bad), (&bad, &ordinary)] {
            let report = validate_bytes(&[first.as_slice(), second.as_slice()].concat());
            assert!(!concatenated(&report), "{label}: {report:?}");
            assert!(noted(&report), "{label}: {report:?}");
            assert_ne!(report.verdict, Verdict::Pass, "{label}: {report:?}");
        }
    }
}
