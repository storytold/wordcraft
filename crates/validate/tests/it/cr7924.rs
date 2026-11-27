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
    /// A ZIP64-shaped record: saturated compressed size and no extra field to resolve it.
    Wide,
    /// A ZIP64-shaped record whose extra field resolves, to an offset that holds a different entry.
    WideResolved,
}

const FAKES: [Fake; 4] = [Fake::Zeroed, Fake::Copied, Fake::Wide, Fake::WideResolved];

/// A first archive whose entry data holds an EOCD-like record declaring a `long` comment that
/// ends `tail` bytes before the end of the `rest`-extended file; returns (first archive, whole file).
fn with_fake_footer(fake: Fake, rest: &[u8], tail: i64) -> (Vec<u8>, Vec<u8>) {
    let plain = raw_zip::build(&records(Doc::minimal()));
    let central = match fake {
        Fake::Zeroed => [b"PK\x01\x02".to_vec(), vec![0; 42]].concat(),
        Fake::Copied => first_central_record(&plain),
        Fake::Wide | Fake::WideResolved => {
            let mut record = [b"PK\x01\x02".to_vec(), vec![0; 42]].concat();
            record[20..24].copy_from_slice(&u32::MAX.to_le_bytes());
            record[28..30].copy_from_slice(&1u16.to_le_bytes());
            record.push(b'a');
            if matches!(fake, Fake::WideResolved) {
                record[24..28].copy_from_slice(&u32::MAX.to_le_bytes());
                record[42..46].copy_from_slice(&u32::MAX.to_le_bytes());
                record[30..32].copy_from_slice(&28u16.to_le_bytes());
                record.extend([1, 0, 24, 0]);
                record.extend([0; 24]);
            }
            record
        }
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
    for fake in FAKES {
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
    for fake in FAKES {
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
    for fake in FAKES {
        let (first, whole) = with_fake_footer(fake, &second, (second.len() + 40) as i64);
        assert!(whole.len() > first.len());
        let report = validate_bytes(&whole);
        assert!(concatenated(&report) || noted(&report), "{report:?}");
        assert_ne!(report.verdict, Verdict::Pass, "{report:?}");
    }
}

#[test]
fn a_fake_footer_alone_is_not_a_concatenation() {
    for fake in FAKES {
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

/// How an archive is written: the size representation in local headers and in the directory, and a
/// prefix that shifts every offset (offsets stay measured from byte zero of the whole file).
#[derive(Clone, Copy, Default)]
struct Shape {
    local_zip64: bool,
    central_zip64: bool,
    stub: usize,
}

fn shaped(doc: Doc, shape: Shape) -> Vec<u8> {
    let mut out = vec![0u8; shape.stub];
    let mut central = Vec::new();
    let count = doc.entries.len();
    for (name, body) in doc.entries {
        let (name, body) = (name.as_bytes().to_vec(), body.to_vec());
        let offset = out.len() as u64;
        let size = body.len() as u64;
        let crc = raw_zip::crc32(&body);
        let local_extra = if shape.local_zip64 { [&[1u8, 0, 16, 0][..], &size.to_le_bytes(), &size.to_le_bytes()].concat() } else { Vec::new() };
        let sat = |wide: bool, v: u64| if wide { u32::MAX } else { v as u32 };
        out.extend(0x04034b50u32.to_le_bytes());
        for value in [20u16, 0, 0, 0, 0] {
            out.extend(value.to_le_bytes());
        }
        out.extend(crc.to_le_bytes());
        out.extend(sat(shape.local_zip64, size).to_le_bytes());
        out.extend(sat(shape.local_zip64, size).to_le_bytes());
        out.extend((name.len() as u16).to_le_bytes());
        out.extend((local_extra.len() as u16).to_le_bytes());
        out.extend(&name);
        out.extend(&local_extra);
        out.extend(&body);
        let central_extra = if shape.central_zip64 {
            [&[1u8, 0, 24, 0][..], &size.to_le_bytes(), &size.to_le_bytes(), &offset.to_le_bytes()].concat()
        } else {
            Vec::new()
        };
        central.extend(0x02014b50u32.to_le_bytes());
        for value in [20u16, 20, 0, 0, 0, 0] {
            central.extend(value.to_le_bytes());
        }
        central.extend(crc.to_le_bytes());
        central.extend(sat(shape.central_zip64, size).to_le_bytes());
        central.extend(sat(shape.central_zip64, size).to_le_bytes());
        for value in [name.len() as u16, central_extra.len() as u16, 0, 0, 0] {
            central.extend(value.to_le_bytes());
        }
        central.extend(0u32.to_le_bytes());
        central.extend(sat(shape.central_zip64, offset).to_le_bytes());
        central.extend(&name);
        central.extend(&central_extra);
    }
    let start = out.len() as u64;
    out.extend(&central);
    if shape.central_zip64 {
        let at = out.len() as u64;
        out.extend(0x06064b50u32.to_le_bytes());
        out.extend(44u64.to_le_bytes());
        out.extend([45u8, 0, 45, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        for value in [count as u64, count as u64, central.len() as u64, start] {
            out.extend(value.to_le_bytes());
        }
        out.extend(0x07064b50u32.to_le_bytes());
        out.extend(0u32.to_le_bytes());
        out.extend(at.to_le_bytes());
        out.extend(1u32.to_le_bytes());
        out.extend(0x06054b50u32.to_le_bytes());
        out.extend([0u8; 4]);
        out.extend(u16::MAX.to_le_bytes());
        out.extend(u16::MAX.to_le_bytes());
        out.extend(u32::MAX.to_le_bytes());
        out.extend(u32::MAX.to_le_bytes());
    } else {
        out.extend(0x06054b50u32.to_le_bytes());
        out.extend([0u8; 4]);
        out.extend((count as u16).to_le_bytes());
        out.extend((count as u16).to_le_bytes());
        out.extend((central.len() as u32).to_le_bytes());
        out.extend((start as u32).to_le_bytes());
    }
    out.extend(0u16.to_le_bytes());
    out
}

const LOCAL64: Shape = Shape { local_zip64: true, central_zip64: false, stub: 0 };
const FULL64: Shape = Shape { local_zip64: true, central_zip64: true, stub: 0 };
const STUBBED: Shape = Shape { local_zip64: false, central_zip64: false, stub: 128 };
const STUBBED_FULL64: Shape = Shape { local_zip64: true, central_zip64: true, stub: 128 };

fn pair(first: &[u8], second: &[u8]) -> Report {
    validate_bytes(&[first, second].concat())
}

#[test]
fn shaped_archives_alone_pass_clean() {
    for (label, shape) in [("local zip64", LOCAL64), ("full zip64", FULL64), ("stub", STUBBED), ("stub + zip64", STUBBED_FULL64)] {
        let report = validate_bytes(&shaped(Doc::minimal(), shape));
        assert!(!concatenated(&report) && !noted(&report), "{label}: {report:?}");
        assert_eq!(report.verdict, Verdict::Pass, "{label}: {report:?}");
    }
}

#[test]
fn a_local_zip64_archive_followed_by_an_ordinary_archive_is_concatenated() {
    let ordinary = Doc::minimal().build();
    for (label, shape) in [("local zip64", LOCAL64), ("full zip64", FULL64)] {
        let first = shaped(Doc::minimal(), shape);
        for report in [pair(&first, &ordinary), pair(&first, &first)] {
            assert!(concatenated(&report), "{label}: {report:?}");
            assert_eq!(report.verdict, Verdict::Fail, "{label}: {report:?}");
        }
    }
}

#[test]
fn a_stub_prefixed_archive_followed_by_an_ordinary_archive_is_concatenated() {
    let ordinary = Doc::minimal().build();
    for (label, shape) in [("stub", STUBBED), ("stub + zip64", STUBBED_FULL64)] {
        let first = shaped(Doc::minimal(), shape);
        let report = pair(&first, &ordinary);
        assert!(concatenated(&report), "{label}: {report:?}");
        assert_eq!(report.verdict, Verdict::Fail, "{label}: {report:?}");
    }
}

// ---- Data descriptors: the directory alone never establishes one, and slack is never granted unproven. ----

#[derive(Clone, Copy)]
enum Descriptor {
    Signed32,
    Bare32,
    Signed64,
    Bare64,
}

const DESCRIPTORS: [(&str, Descriptor); 4] =
    [("signed", Descriptor::Signed32), ("bare", Descriptor::Bare32), ("signed zip64", Descriptor::Signed64), ("bare zip64", Descriptor::Bare64)];

/// A streamed archive: every local header defers its checksum and sizes to a descriptor after the data.
fn streamed(doc: Doc, kind: Descriptor) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    let count = doc.entries.len();
    for (name, body) in doc.entries {
        let (name, body) = (name.as_bytes().to_vec(), body.to_vec());
        let offset = out.len() as u32;
        let (crc, size) = (raw_zip::crc32(&body), body.len() as u32);
        out.extend(0x04034b50u32.to_le_bytes());
        for value in [20u16, 8, 0, 0, 0] {
            out.extend(value.to_le_bytes());
        }
        out.extend([0u8; 12]);
        out.extend((name.len() as u16).to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out.extend(&name);
        out.extend(&body);
        if matches!(kind, Descriptor::Signed32 | Descriptor::Signed64) {
            out.extend(0x08074b50u32.to_le_bytes());
        }
        out.extend(crc.to_le_bytes());
        if matches!(kind, Descriptor::Signed32 | Descriptor::Bare32) {
            out.extend(size.to_le_bytes());
            out.extend(size.to_le_bytes());
        } else {
            out.extend(u64::from(size).to_le_bytes());
            out.extend(u64::from(size).to_le_bytes());
        }
        central.extend(0x02014b50u32.to_le_bytes());
        for value in [20u16, 20, 8, 0, 0, 0] {
            central.extend(value.to_le_bytes());
        }
        for value in [crc, size, size] {
            central.extend(value.to_le_bytes());
        }
        for value in [name.len() as u16, 0, 0, 0, 0] {
            central.extend(value.to_le_bytes());
        }
        central.extend(0u32.to_le_bytes());
        central.extend(offset.to_le_bytes());
        central.extend(&name);
    }
    let start = out.len() as u32;
    out.extend(&central);
    out.extend(0x06054b50u32.to_le_bytes());
    out.extend([0u8; 4]);
    out.extend((count as u16).to_le_bytes());
    out.extend((count as u16).to_le_bytes());
    out.extend((central.len() as u32).to_le_bytes());
    out.extend(start.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out
}

#[test]
fn streamed_archives_with_real_descriptors_pass_alone_and_still_concatenate() {
    let ordinary = Doc::minimal().build();
    for (label, kind) in DESCRIPTORS {
        let first = streamed(Doc::minimal(), kind);
        let alone = validate_bytes(&first);
        assert!(!concatenated(&alone) && !noted(&alone), "{label}: {alone:?}");
        assert_eq!(alone.verdict, Verdict::Pass, "{label}: {alone:?}");
        for report in [pair(&first, &ordinary), pair(&ordinary, &first)] {
            assert!(concatenated(&report), "{label}: {report:?}");
            assert_eq!(report.verdict, Verdict::Fail, "{label}: {report:?}");
        }
    }
}

/// An archive whose first entry is a blob holding a record set that claims a directory of one entry: the
/// blob itself, with a data descriptor the central record asserts. `local_flag` says whether the blob's
/// real local header agrees; `pad` bytes separate the claimed entry end from the claimed directory.
/// The fake footer's comment runs 5 bytes short of the end of `rest`, across the real archive boundary.
fn descriptor_decoy(rest: &[u8], local_flag: bool, pad: usize, pad_byte: u8) -> (Vec<u8>, Vec<u8>) {
    const NAME: &str = "word/media/blob.bin";
    let blob_at = 30 + NAME.len();
    let make = |comment: u16| {
        let mut blob = vec![pad_byte; pad];
        let directory = blob_at + pad;
        let mut central = 0x02014b50u32.to_le_bytes().to_vec();
        for value in [20u16, 20, 8, 0, 0, 0] {
            central.extend(value.to_le_bytes());
        }
        central.extend([0u8; 12]);
        for value in [NAME.len() as u16, 0, 0, 0, 0] {
            central.extend(value.to_le_bytes());
        }
        central.extend([0u8; 4]);
        central.extend(0u32.to_le_bytes());
        central.extend(NAME.as_bytes());
        blob.extend(&central);
        blob.extend(0x06054b50u32.to_le_bytes());
        for value in [0u16, 0, 1, 1] {
            blob.extend(value.to_le_bytes());
        }
        blob.extend((central.len() as u32).to_le_bytes());
        blob.extend((directory as u32).to_le_bytes());
        blob.extend(comment.to_le_bytes());
        let mut recs = vec![Record::new(NAME, 0, &blob)];
        recs.extend(records(Doc::minimal()));
        let mut zip = raw_zip::build(&recs);
        if local_flag {
            zip[6] |= 8;
        }
        zip
    };
    let probe = make(0);
    let fake_footer = blob_at + pad + 46 + NAME.len();
    let total = probe.len() + rest.len();
    let comment = u16::try_from(total - (fake_footer + 22) - 5).unwrap();
    let first = make(comment);
    assert_eq!(first.len(), probe.len());
    let whole = [first.as_slice(), rest].concat();
    (first, whole)
}

#[test]
fn a_central_descriptor_claim_the_local_header_lacks_cannot_hide_a_boundary() {
    let second = Doc::minimal().build();
    for pad in [0, 10, 24] {
        let (_, whole) = descriptor_decoy(&second, false, pad, 0);
        let report = validate_bytes(&whole);
        assert!(concatenated(&report), "pad {pad}: {report:?}");
        assert_eq!(report.verdict, Verdict::Fail, "pad {pad}: {report:?}");
    }
}

#[test]
fn a_descriptor_both_records_claim_but_the_bytes_do_not_hold_cannot_hide_a_boundary() {
    let second = Doc::minimal().build();
    for pad in [0, 10, 24] {
        let (_, whole) = descriptor_decoy(&second, true, pad, 0xA5);
        let report = validate_bytes(&whole);
        assert!(concatenated(&report) || noted(&report), "pad {pad}: {report:?}");
        assert_ne!(report.verdict, Verdict::Pass, "pad {pad}: {report:?}");
    }
}

#[test]
fn a_local_descriptor_flag_the_directory_lacks_still_leaves_the_archive_splittable() {
    let ordinary = Doc::minimal().build();
    let mut first = Doc::minimal().build();
    first[6] |= 8;
    let report = pair(&first, &ordinary);
    assert!(concatenated(&report) || noted(&report), "{report:?}");
    assert_ne!(report.verdict, Verdict::Pass, "{report:?}");
}

/// `zip` with `comment` as its archive comment.
fn with_comment(mut zip: Vec<u8>, comment: &[u8]) -> Vec<u8> {
    let at = zip.len() - 2;
    zip[at..].copy_from_slice(&u16::try_from(comment.len()).unwrap().to_le_bytes());
    zip.extend(comment);
    zip
}

/// An EOCD-like record with one 46-byte directory at `start`.
fn implausible_footer(start: u32) -> Vec<u8> {
    let mut record = b"PK\x05\x06".to_vec();
    for value in [0u16, 0, 1, 1] {
        record.extend(value.to_le_bytes());
    }
    record.extend(46u32.to_le_bytes());
    record.extend(start.to_le_bytes());
    record.extend(0u16.to_le_bytes());
    record
}

/// A lookalike in the last comment whose directory start is beyond itself (no reader accepts it) or zero (a
/// local header, which the reader still treats as a competing footer) must not hide the real second footer.
#[test]
fn a_footer_lookalike_in_the_last_comment_does_not_hide_the_real_second_footer() {
    let ordinary = Doc::minimal().build();
    for start in [1_000_000u32, 0] {
        for padding in [0usize, 10] {
            let comment = [vec![b'y'; padding], implausible_footer(start)].concat();
            let second = with_comment(ordinary.clone(), &comment);
            let report = validate_bytes(&[ordinary.as_slice(), second.as_slice()].concat());
            assert!(concatenated(&report), "start {start} padding {padding}: {report:?}");
            assert_eq!(report.verdict, Verdict::Fail, "start {start} padding {padding}: {report:?}");
        }
        // A single archive carrying the same lookalike has no second archive to find.
        let single = validate_bytes(&with_comment(ordinary.clone(), &implausible_footer(start)));
        assert!(!concatenated(&single), "start {start}: {single:?}");
        if start == 0 {
            assert_eq!(single.verdict, Verdict::Incomplete, "{single:?}");
        }
    }
}

/// Identical slices share one load, which must not skip the relationships check the second archive needs.
#[test]
fn identical_archives_with_a_malformed_relationships_part_are_not_certified_complete() {
    let mut doc = Doc::minimal();
    doc.set("_rels/.rels", [package_rels_xml(), b"trailing text".to_vec()].concat());
    let bad = doc.build();
    let report = validate_bytes(&[bad.as_slice(), bad.as_slice()].concat());
    assert!(!concatenated(&report), "{report:?}");
    assert!(report.findings.iter().any(|f| f.rule == "XML-MALFORMED"), "{report:?}");
    assert_eq!(report.verdict, Verdict::Fail, "{report:?}");
}

#[test]
fn a_second_archive_with_a_malformed_relationships_part_is_not_certified_complete() {
    let ordinary = Doc::minimal().build();
    let control = validate_bytes(&[ordinary.as_slice(), ordinary.as_slice()].concat());
    assert!(concatenated(&control), "{control:?}");
    for trailer in ["<Relationships/>", "trailing text"] {
        let mut doc = Doc::minimal();
        doc.set("_rels/.rels", [package_rels_xml(), trailer.as_bytes().to_vec()].concat());
        let bad = doc.build();
        let report = validate_bytes(&[ordinary.as_slice(), bad.as_slice()].concat());
        assert!(!concatenated(&report), "{trailer}: {report:?}");
        assert!(noted(&report), "{trailer}: {report:?}");
        assert_eq!(report.verdict, Verdict::Incomplete, "{trailer}: {report:?}");
    }
}

/// One entry `a` whose directory and local header both claim a data descriptor, with a ZIP64 compressed
/// size of `size` (so the descriptor is looked for `size` bytes into the file) and `comment` as the archive
/// comment. A comment makes the footer an anchor candidate, so the scan walks its directory instead of skipping it.
fn descriptor_claim_with(size: u64, descriptor: bool, zip64: bool, comment: &[u8]) -> Vec<u8> {
    let flags: u16 = if descriptor { 8 } else { 0 };
    let mut out = b"PK\x03\x04".to_vec();
    for value in [20u16, flags, 0, 0, 0] {
        out.extend(value.to_le_bytes());
    }
    for value in [0u32, 0, 0] {
        out.extend(value.to_le_bytes());
    }
    for value in [1u16, 0] {
        out.extend(value.to_le_bytes());
    }
    out.push(b'a');
    out.extend(b"xyz");
    let start = out.len();
    let extra = if zip64 { [&[1u8, 0, 8, 0][..], &size.to_le_bytes()].concat() } else { Vec::new() };
    let sized = if zip64 { u32::MAX } else { size as u32 };
    let mut central = b"PK\x01\x02".to_vec();
    for value in [20u16, 20, flags, 0, 0, 0] {
        central.extend(value.to_le_bytes());
    }
    for value in [0u32, sized, 0] {
        central.extend(value.to_le_bytes());
    }
    for value in [1u16, extra.len() as u16, 0, 0, 0] {
        central.extend(value.to_le_bytes());
    }
    central.extend([0u8; 8]);
    central.push(b'a');
    central.extend(&extra);
    out.extend(&central);
    out.extend(b"PK\x05\x06");
    for value in [0u16, 0, 1, 1] {
        out.extend(value.to_le_bytes());
    }
    out.extend((central.len() as u32).to_le_bytes());
    out.extend((start as u32).to_le_bytes());
    out.extend(u16::try_from(comment.len()).unwrap().to_le_bytes());
    out.extend(comment);
    out
}

#[test]
fn a_data_descriptor_far_beyond_the_file_never_overflows_the_offset_arithmetic() {
    // Run in an overflow-checked build, an offset that wraps would panic; it must only mean "no descriptor".
    let end_of_space = u64::MAX - 1 - 31;
    for comment in [&b""[..], b"c"] {
        let control = validate_bytes(&descriptor_claim_with(3, true, true, comment));
        let huge = validate_bytes(&descriptor_claim_with(end_of_space, true, true, comment));
        assert_eq!(huge.verdict, control.verdict, "comment {comment:?}: {huge:?}");
        assert_ne!(huge.verdict, Verdict::Pass);
    }
}

/// Hostile-input sweep for the footer walk: descriptor claims at every size near the ends of the address
/// space and around the file length never panic and never pass.
#[test]
fn descriptor_size_sweep_never_panics_or_passes() {
    let edges = [0u64, 1, 3, 4, 31, 32, u32::MAX as u64 - 1, u32::MAX as u64, 1 << 32, 1 << 40, 1 << 62, u64::MAX / 2];
    for base in edges.into_iter().chain((0..64).map(|k| u64::MAX - k)) {
        for (descriptor, zip64) in [(true, true), (true, false), (false, true)] {
            // The nonempty comment sends the footer through the directory walk.
            for comment in [&b""[..], b"c"] {
                let report = validate_bytes(&descriptor_claim_with(base, descriptor, zip64, comment));
                assert_ne!(report.verdict, Verdict::Pass, "size {base} descriptor {descriptor} zip64 {zip64} comment {comment:?}: {report:?}");
            }
        }
    }
}
