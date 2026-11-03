//! A bytes-only check of a ZIP package's declared entry count before the `zip` crate opens it
//! (#3660). Every reader of `.docx`, `.odt` and spreadsheet packages calls [`check`] first.
//!
//! `zip::ZipArchive::new` reserves room for, and parses, as many central directory headers as the
//! end record declares before its caller can look at `len()`. A ZIP64 end record can declare
//! millions of entries for a file of a few hundred megabytes (only 46 bytes each have to fit), and
//! each parsed entry costs several times that in memory. Here the end records are found the way
//! `zip` finds them (APPNOTE.TXT §4.3.14 to §4.3.16), and a package declaring more than the
//! reader's limit is refused before `zip` parses anything. Anything else that is wrong is left for
//! `zip` to report.

const EOCD: &[u8; 4] = b"PK\x05\x06";
const EOCD64_LOCATOR: &[u8; 4] = b"PK\x06\x07";
const EOCD64: &[u8; 4] = b"PK\x06\x06";
const CENTRAL_HEADER: &[u8; 4] = b"PK\x01\x02";

/// End records looked at, at most. A package has one; a few more can turn up inside stored data.
const MAX_END_RECORDS: usize = 16;

/// Refuse `bytes` when the end record `zip` would use declares more than `max_entries` entries.
///
/// `zip` searches the file backwards for an end of central directory record. A ZIP64 one points
/// through its locator at the ZIP64 end record, which `zip` looks for from the locator's offset
/// on: every such record there is checked, and as `zip` may fall back to an earlier end record
/// when one doesn't parse, the search goes on. A plain one is used when it declares no entries, or
/// a central directory header lies between its directory offset and itself: it is the last one
/// looked at.
pub fn check(bytes: &[u8], max_entries: u64) -> Result<(), String> {
    let u16_at = |at: usize| bytes.get(at..at.checked_add(2)?).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |at: usize| bytes.get(at..at.checked_add(4)?).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let u64_at = |at: usize| bytes.get(at..at.checked_add(8)?).and_then(|b| <[u8; 8]>::try_from(b).ok()).map(u64::from_le_bytes);
    let refuse = |n: u64| -> Result<(), String> {
        if n > max_entries { Err(format!("the package declares {n} entries, more than {max_entries}")) } else { Ok(()) }
    };
    let mut end = bytes.len();
    let mut seen = 0;
    while let Some(at) = bytes.get(..end).and_then(|b| b.windows(4).rposition(|w| w == EOCD)) {
        end = at;
        seen += 1;
        if seen > MAX_END_RECORDS {
            return Err(format!("more than {MAX_END_RECORDS} end of central directory records"));
        }
        // `zip` skips a record whose comment runs past the end of the file.
        let Some(comment) = u16_at(at + 20) else { continue };
        if at + 22 + usize::from(comment) > bytes.len() {
            continue;
        }
        let (Some(on_disk), Some(total), Some(cd_offset)) = (u16_at(at + 8), u16_at(at + 10), u32_at(at + 16)) else { continue };
        let zip64 = total == u16::MAX || cd_offset == u32::MAX;
        if zip64
            && let Some(locator) = at.checked_sub(20)
            && bytes.get(locator..locator + 4) == Some(EOCD64_LOCATOR.as_slice())
        {
            let (Some(eocd64), Some(disks)) = (u64_at(locator + 8), u32_at(locator + 16)) else { continue };
            let Some(from) = usize::try_from(eocd64).ok().filter(|&f| f < locator) else { continue };
            if disks > 1 {
                continue;
            }
            let records = bytes.get(from..locator).unwrap_or_default().windows(4).enumerate().filter(|(_, w)| *w == EOCD64);
            for (i, _) in records {
                let p = from + i;
                refuse(u64_at(p + 24).unwrap_or(0).max(u64_at(p + 32).unwrap_or(0)))?;
            }
            continue;
        }
        if total == 0 {
            return Ok(());
        }
        let cd = cd_offset as usize;
        if cd < at && bytes.get(cd..at).is_some_and(|b| b.windows(4).any(|w| w == CENTRAL_HEADER)) {
            return refuse(u64::from(on_disk.max(total)));
        }
    }
    Ok(())
}

/// A package of one empty stored file, `a`, whose central directory lists it `entries` times (at
/// most as many as fit in 64 MiB) under a ZIP64 end record that declares them all: `zip` 3.0.0
/// parses and keeps every one of them before its caller can count them. Regression input for the
/// readers' tests: built only for tests, this crate's and (through the `test-support` feature,
/// which they turn on as a dev-dependency) the readers', never into the app.
#[cfg(any(test, feature = "test-support"))]
pub fn zip64_entry_count_sample(entries: u64) -> Vec<u8> {
    // Local file header (APPNOTE.TXT §4.3.7): version 10, stored, no data, name "a".
    let mut f = b"PK\x03\x04".to_vec();
    f.extend_from_slice(&[10, 0, 0, 0, 0, 0, 0, 0, 0x21, 0]); // version, flags, method, time, date
    f.extend_from_slice(&[0; 12]); // crc, sizes
    f.extend_from_slice(&[1, 0, 0, 0]); // name and extra lengths
    f.push(b'a');
    let cd_at = f.len() as u64;
    // Its central header (§4.3.12), 47 bytes.
    let mut header = CENTRAL_HEADER.to_vec();
    header.extend_from_slice(&[10, 0, 10, 0, 0, 0, 0, 0, 0, 0, 0x21, 0]); // versions, flags, method, time, date
    header.extend_from_slice(&[0; 12]); // crc, sizes
    header.extend_from_slice(&[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]); // name, extra, comment, disk, attributes
    header.extend_from_slice(&[0; 4]); // local header offset
    header.push(b'a');
    let count = entries.min((64 << 20) / header.len() as u64);
    for _ in 0..count {
        f.extend_from_slice(&header);
    }
    let cd_len = f.len() as u64 - cd_at;
    let eocd64_at = f.len() as u64;
    // ZIP64 end of central directory record (§4.3.14), 56 bytes.
    f.extend_from_slice(EOCD64);
    f.extend_from_slice(&44u64.to_le_bytes()); // size of the rest of the record
    f.extend_from_slice(&[45, 0, 45, 0]); // versions
    f.extend_from_slice(&[0; 8]); // this disk, disk with the central directory
    f.extend_from_slice(&entries.to_le_bytes()); // entries on this disk
    f.extend_from_slice(&entries.to_le_bytes()); // entries
    f.extend_from_slice(&cd_len.to_le_bytes());
    f.extend_from_slice(&cd_at.to_le_bytes());
    // Its locator (§4.3.15), 20 bytes.
    f.extend_from_slice(EOCD64_LOCATOR);
    f.extend_from_slice(&0u32.to_le_bytes());
    f.extend_from_slice(&eocd64_at.to_le_bytes());
    f.extend_from_slice(&1u32.to_le_bytes());
    // End of central directory record (§4.3.16), 22 bytes, each count and offset "see ZIP64".
    f.extend_from_slice(EOCD);
    f.extend_from_slice(&[0; 4]);
    f.extend_from_slice(&[0xFF; 12]);
    f.extend_from_slice(&[0; 2]);
    f
}

#[cfg(test)]
mod tests {
    use super::{CENTRAL_HEADER, EOCD, check, zip64_entry_count_sample};

    /// A plain package's tail: one central header at 0, then an end record declaring `entries`.
    fn plain(entries: u16, comment: &[u8]) -> Vec<u8> {
        let mut f = CENTRAL_HEADER.to_vec();
        f.resize(46, 0);
        f.extend_from_slice(EOCD);
        f.extend_from_slice(&[0, 0, 0, 0]);
        f.extend_from_slice(&entries.to_le_bytes());
        f.extend_from_slice(&entries.to_le_bytes());
        f.extend_from_slice(&46u32.to_le_bytes());
        f.extend_from_slice(&0u32.to_le_bytes());
        f.extend_from_slice(&(comment.len() as u16).to_le_bytes());
        f.extend_from_slice(comment);
        f
    }

    #[test]
    fn a_zip64_end_record_declaring_too_many_entries_is_refused() {
        // #3660: `zip` would reserve and parse them all before the reader's cap.
        let sample = zip64_entry_count_sample(20_001);
        assert!(check(&sample, 20_000).is_err_and(|e| e.contains("20001 entries")));
        assert!(check(&zip64_entry_count_sample(5), 20_000).is_ok());
        // Junk before the package (a self-extracting stub) doesn't hide it.
        let mut prefixed = vec![0x55u8; 4096];
        prefixed.extend_from_slice(&sample);
        assert!(check(&prefixed, 20_000).is_err());
    }

    #[test]
    fn a_plain_end_record_is_checked_as_zip_reads_it() {
        assert!(check(&plain(3, b""), 20_000).is_ok());
        assert!(check(&plain(30_000, b"a comment"), 20_000).is_err_and(|e| e.contains("30000 entries")));
        // A record whose comment runs past the end is not the one `zip` uses.
        let mut cut = plain(30_000, b"");
        cut.truncate(cut.len() - 2);
        cut.extend_from_slice(&500u16.to_le_bytes());
        assert!(check(&cut, 20_000).is_ok());
        assert!(check(b"not a package", 20_000).is_ok());
        assert!(check(b"", 20_000).is_ok());
    }

    #[test]
    fn a_fake_end_record_after_the_real_one_does_not_hide_it() {
        // `zip` skips a last record with no central header before it and uses the one before.
        let mut f = zip64_entry_count_sample(1_000_000);
        f.extend_from_slice(EOCD);
        f.extend_from_slice(&[0, 0, 0, 0]);
        f.extend_from_slice(&7u16.to_le_bytes());
        f.extend_from_slice(&7u16.to_le_bytes());
        f.extend_from_slice(&0u32.to_le_bytes());
        f.extend_from_slice(&((f.len() - 12) as u32).to_le_bytes());
        f.extend_from_slice(&0u16.to_le_bytes());
        assert!(check(&f, 20_000).is_err());
    }

    #[test]
    fn many_end_records_are_refused() {
        // Each declares one entry and has no central header before it, so `zip` would try them all.
        let record = [0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let f: Vec<u8> = (0..20).flat_map(|_| EOCD.iter().copied().chain(record)).collect();
        assert!(check(&f, 20_000).is_err_and(|e| e.contains("end of central directory records")));
    }
}
