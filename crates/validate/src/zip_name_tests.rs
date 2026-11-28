use super::*;
#[path = "../tests/common/raw_zip.rs"]
mod raw_zip;
use raw_zip::{Record, build};

/// H6-P12: the independent ZIP reader is the oracle for every decoding branch.
#[test]
fn h6_p12_decoder_matches_zip_file_name() {
    let mut records = vec![
        Record::new(b"f\x80.xml", 0, b""),
        Record::new(b"f\x81.xml", 0, b""),
        Record::new("fü.xml", 0x800, b""),
        Record::new(b"f\xff\xff.xml", 0x800, b""),
        Record::new("f�.xml", 0x800, b""),
        Record::new(b"a", 0, b"").unicode(1, b"a", "α".as_bytes()),
        Record::new(b"a", 0, b"").unicode(7, b"a", "β".as_bytes()),
        Record::new("é", 0x800, b"").unicode(1, "é".as_bytes(), "γ".as_bytes()),
        Record::new(b"a", 0, b"").unicode(1, b"a", b"b").unicode(7, b"b", "δ".as_bytes()),
    ];
    // Pin the entire CP437 table, including ASCII/control bytes and its box-drawing range.
    records.extend((0u8..=255).map(|b| Record::new([b], 0, b"")));
    for record in records {
        let bytes = build(std::slice::from_ref(&record));
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let file = archive.by_index(0).unwrap();
        assert_eq!(decode_zip_name(&record.name, record.flags & 0x800 != 0, &record.extra).unwrap(), file.name());
    }
    assert_eq!(name_crc32(b"123456789"), 0xcbf43926);
}

#[test]
fn unicode_path_rejects_bad_crc_and_utf8_like_zip() {
    let mut bad_crc = Record::new(b"a", 0, b"").unicode(1, b"a", b"b");
    bad_crc.extra[5] ^= 1;
    for r in [bad_crc, Record::new(b"a", 0, b"").unicode(1, b"a", b"\xff")] {
        assert!(decode_zip_name(&r.name, false, &r.extra).is_none());
        assert!(zip::ZipArchive::new(Cursor::new(build(&[r]))).is_err());
    }
}

#[test]
fn directory_count_residual_is_signed_and_ignores_truncation() {
    assert_eq!(directory_residual(6, 4, 2, false), Some(0));
    assert_eq!(directory_residual(7, 4, 2, false), Some(1));
    assert_eq!(directory_residual(5, 4, 2, false), Some(-1));
    assert_eq!(directory_residual(7, 4, 2, true), None);
    assert_eq!(directory_residual(5, 4, 2, true), None);
}

fn eocd(count: u16, start: u32) -> Vec<u8> {
    let mut record = b"PK\x05\x06".to_vec();
    for value in [0u16, 0, count, count] {
        record.extend(value.to_le_bytes());
    }
    record.extend(0u32.to_le_bytes());
    record.extend(start.to_le_bytes());
    record.extend(0u16.to_le_bytes());
    record
}

fn named(count: usize) -> Vec<Record> {
    (0..count).map(|i| Record::new(format!("f{i}"), 0, b"x")).collect()
}

/// Footer-like records aimed at one directory share one walk; records aimed at ever-new directories are
/// charged against the scan-wide budget, which then reports itself spent instead of walking on.
#[test]
fn footer_walks_are_shared_per_directory_and_bounded_in_total() {
    let real = build(&named(30));
    let footer = real.len() - 22;
    let start = u32::from_le_bytes(real[footer + 16..footer + 20].try_into().unwrap());

    let mut same = FooterWalks::new(40);
    let mut bytes = real.clone();
    let mut footers = Vec::new();
    for _ in 0..200 {
        footers.push(bytes.len());
        bytes.extend(eocd(30, start));
    }
    for at in footers {
        assert!(matches!(same.footer(&bytes, at, 40), Footer::Verified(30)));
    }
    assert!(!same.spent, "one directory is walked once however many records point at it");

    // Different counts are different directories: each is a fresh walk.
    let mut distinct = FooterWalks::new(40);
    let mut bytes = real.clone();
    let mut footers = Vec::new();
    for count in 1..=30 {
        footers.push(bytes.len());
        bytes.extend(eocd(count, start));
    }
    for at in &footers {
        distinct.footer(&bytes, *at, 40);
    }
    assert!(distinct.spent);
    // Once spent, no verdict of "verified" is invented for a directory that was not walked.
    assert!(matches!(distinct.footer(&bytes, footers[29], 40), Footer::Unanchored(30)));
}

/// Offsets that wrap past `usize::MAX` are absent, not a panic in an overflow-checked build.
#[test]
fn byte_readers_and_descriptor_lengths_survive_offsets_at_the_end_of_the_address_space() {
    let bytes = [0u8; 64];
    for at in [usize::MAX, usize::MAX - 1, usize::MAX - 3, usize::MAX - 7, usize::MAX - 11, usize::MAX - 15] {
        assert_eq!((u16_at(&bytes, at), u32_at(&bytes, at), u64_at(&bytes, at)), (None, None, None));
        assert_eq!(descriptor_lengths(&bytes, at, 0, 0, 0), 0);
    }
}

mod trailing_budget {
    use super::raw_zip::{self, Record};
    use super::*;
    use crate::resolution_tests::common::Doc;
    use crate::{Limits, Verdict, validate_bytes_limited};

    const NOTE: &str = "could not resolve a candidate boundary";

    fn concatenated(report: &crate::Report) -> bool {
        report.findings.iter().any(|f| f.rule == "PKG-CONCATENATED")
    }

    fn with_comment(mut zip: Vec<u8>, comment: &[u8]) -> Vec<u8> {
        let at = zip.len() - 2;
        zip[at..].copy_from_slice(&u16::try_from(comment.len()).unwrap().to_le_bytes());
        zip.extend(comment);
        zip
    }

    fn footer_copy_with_count(zip: &[u8], count: u16) -> Vec<u8> {
        let mut copy = zip[zip.len() - 22..].to_vec();
        copy[8..10].copy_from_slice(&count.to_le_bytes());
        copy[10..12].copy_from_slice(&count.to_le_bytes());
        copy
    }

    /// The ordinary archive plus a stored entry holding `fakes` footer-like records whose comments run to the
    /// end of the file once `tail` more bytes follow it, so each is an exact-end candidate below the real footer.
    fn archive_with_inner_candidates(fakes: usize, tail: usize) -> Vec<u8> {
        let build = |blob: &[u8]| {
            let mut records: Vec<Record> = Doc::minimal().entries.into_iter().map(|(name, body)| Record::new(name, 0, body)).collect();
            records.push(Record::new("word/media/blob.bin", 0, blob));
            raw_zip::build(&records)
        };
        let record = |comment: u16| [b"PK\x05\x06".as_slice(), &[0u8; 16], &comment.to_le_bytes()].concat();
        let probe = build(&vec![0u8; fakes * 22]);
        let name = b"word/media/blob.bin";
        let data = probe.windows(name.len()).position(|w| w == name).unwrap() + name.len();
        let total = probe.len() + tail;
        let blob: Vec<u8> = (0..fakes).flat_map(|at| record(u16::try_from(total - (data + at * 22 + 22)).unwrap())).collect();
        let built = build(&blob);
        assert_eq!(built.len(), probe.len());
        built
    }

    /// An attempt is charged and completed before any successor is searched for. The trailing archive's real
    /// footer is the one that certifies (through the verified view, because a footer copy in its comment misleads
    /// the whole-slice load); candidates below it must not spend the budget its own directory walk needs.
    #[test]
    fn a_successor_lookahead_does_not_spend_the_current_attempts_budget() {
        let first = Doc::minimal().build();
        let entries = 5;
        for fakes in [0usize, 4, 8, 9, 10, 16, 40] {
            let probe = archive_with_inner_candidates(fakes, 0);
            let lookalike = footer_copy_with_count(&probe, 1);
            let second = with_comment(archive_with_inner_candidates(fakes, lookalike.len()), &lookalike);
            let bytes = [first.as_slice(), second.as_slice()].concat();
            let report = validate_bytes_limited(&bytes, Limits { entries, ..Limits::default() });
            assert!(concatenated(&report), "{fakes} candidates below the real footer: {report:?}");
            assert_eq!(report.verdict, Verdict::Fail, "{fakes}: {report:?}");
        }
    }

    /// Whole-slice package loads of one trailing slice share one allowance across candidate attempts, so many
    /// plausible footer copies cannot repeat the extraction and XML work without limit; refusal keeps the note.
    #[test]
    fn candidate_retries_share_one_aggregate_package_load_allowance() {
        let ordinary = Doc::minimal().build();
        let limits = Limits::default();
        // An unproven footer (no comment) loads the whole slice each time it is attempted.
        let footer = ordinary.len() - 22;
        let mut walks = FooterWalks::new(limits.entries);
        for _ in 0..TRAILING_LOADS * 3 {
            standalone_trailing(&ordinary, footer, limits, &mut walks);
        }
        assert_eq!(walks.loads, TRAILING_LOADS, "loads past the allowance are refused, not performed");
        assert!(walks.spent);
        assert_eq!(standalone_trailing(&ordinary, footer, limits, &mut walks), None, "a refused load certifies nothing");

        // Many plausible lookalikes in the comment of an unequal trailing slice: the work stays bounded and the
        // undecided boundary is reported, never passed.
        let lookalike = footer_copy_with_count(&ordinary, 1);
        let copies = TRAILING_LOADS * 2;
        // Each copy's own comment runs to the end of the slice, so every one is an exact-end candidate.
        let chain: Vec<u8> = (0..copies)
            .flat_map(|at| {
                let mut copy = lookalike.clone();
                copy[20..22].copy_from_slice(&u16::try_from((copies - 1 - at) * 22).unwrap().to_le_bytes());
                copy
            })
            .collect();
        let second = with_comment(ordinary.clone(), &chain);
        let report = validate_bytes_limited(&[ordinary.as_slice(), second.as_slice()].concat(), limits);
        assert_ne!(report.verdict, Verdict::Pass, "{report:?}");
        assert!(report.coverage_notes.iter().any(|n| n.contains(NOTE)), "{report:?}");

        // One lookalike is the committed fixture: its disposition is unchanged.
        let one = with_comment(ordinary.clone(), &lookalike);
        let report = validate_bytes_limited(&[ordinary.as_slice(), one.as_slice()].concat(), limits);
        assert!(concatenated(&report), "{report:?}");
        assert_eq!(report.verdict, Verdict::Fail, "{report:?}");
    }
}
