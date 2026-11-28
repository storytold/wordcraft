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
