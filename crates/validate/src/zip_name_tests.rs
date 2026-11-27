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
